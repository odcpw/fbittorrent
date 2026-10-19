//! BEP 3 bitfields: bounded wire data and allocation-free bit tests after decode.
use super::validation::{Checked, Validate, Violation};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pieces {
    pub count: u64,
    pub piece_size: u64,
    pub have: String,
    pub priority: Vec<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub playable_end: Option<u64>,
}
impl Pieces {
    pub fn decode(&self) -> Result<Vec<u8>, Violation> {
        if self.have.len() > super::LINE_OUT || self.count > (super::LINE_OUT as u64) * 6 {
            return Err(Violation("pieces.have: exceeds frame budget".into()));
        }
        let bytes = STANDARD.decode(&self.have).map_err(|_| Violation("pieces.have: invalid base64".into()))?;
        if bytes.len() as u64 != self.count.div_ceil(8) {
            return Err(Violation("pieces.have: byte count must equal ceil(count/8)".into()));
        }
        if self.count % 8 != 0 && bytes.last().is_some_and(|v| v & ((1 << (8 - self.count % 8)) - 1) != 0) {
            return Err(Violation("pieces.have: unused low bits must be zero".into()));
        }
        Ok(bytes)
    }
    pub fn has(bytes: &[u8], index: u64) -> bool {
        bytes.get((index / 8) as usize).is_some_and(|v| v & (0x80 >> (index % 8)) != 0)
    }
}
impl Validate for Pieces {
    fn validate(&self) -> Checked {
        self.decode()?;
        if self.piece_size == 0 { return Err(Violation("pieces.piece_size: must be positive".into())); }
        if self.priority.iter().any(|p| *p >= self.count) || self.priority.windows(2).any(|p| p[0] >= p[1]) {
            return Err(Violation("pieces.priority: sorted unique indices below count required".into()));
        }
        if self.playable_end.is_some_and(|end| end > self.count) {
            return Err(Violation("pieces.playable_end: exceeds piece count".into()));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bitfield_roundtrip() {
        let p=Pieces {count:10,piece_size:16384,have:STANDARD.encode([0x81,0x40]),priority:vec![0,9],playable_end:Some(1)};
        p.validate().unwrap();
        let bytes=p.decode().unwrap();
        assert!(Pieces::has(&bytes,0) && Pieces::has(&bytes,7) && Pieces::has(&bytes,9));
        assert!(!Pieces::has(&bytes,1) && !Pieces::has(&bytes,8));
        assert_eq!(serde_json::from_value::<Pieces>(serde_json::to_value(&p).unwrap()).unwrap(),p);
        for bad in [Pieces {have:"gUE=".into(),..p.clone()},Pieces {have:"gQ==".into(),..p.clone()},
            Pieces {have:"??".into(),..p.clone()},Pieces {priority:vec![10],..p.clone()},
            Pieces {priority:vec![9,0],..p.clone()},Pieces {playable_end:Some(11),..p.clone()}] { assert!(bad.validate().is_err()); }
        println!("MSB-first bits 0,7,9; invalid padding/length/base64/priority/prefix rejected");
    }
    #[test]
    fn frame_under_64k_200000() {
        let p=Pieces {count:200000,piece_size:16384,have:STANDARD.encode(vec![0xa5;25000]),priority:vec![0,199999],playable_end:None};
        p.validate().unwrap();
        let frame=serde_json::to_vec(&serde_json::json!({"ev":"state","sub":1,"topic":format!("torrent:{}.pieces","a".repeat(40)),"rev":1,"data":p})).unwrap();
        println!("200000-piece state frame: {} bytes including newline",frame.len()+1);
        assert!(frame.len()+1 < 65536);
    }
}
