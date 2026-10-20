//! Destination inspection input and output (P6-11).
use serde::{Deserialize, Deserializer, Serialize};
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FreeSpaceArgs {
    #[serde(deserialize_with = "path")]
    pub path: String,
}
fn path<'de, D: Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    let path = String::deserialize(d)?;
    crate::free_space::valid_path(&path).map_err(serde::de::Error::custom)?;
    Ok(path)
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FreeSpaceResult {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub free_bytes: Option<u64>,
    pub writable: bool,
    #[serde(default,skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn input_and_result_are_bounded_and_strict() {
        for value in [
            serde_json::json!({"path":"relative"}),
            serde_json::json!({"path":"/tmp/a","typo":true}),
            serde_json::json!({"path":"/".repeat(4097)}),
        ] {
            assert!(serde_json::from_value::<FreeSpaceArgs>(value).is_err());
        }
        assert!(
            serde_json::from_value::<FreeSpaceArgs>(serde_json::json!({"path":"/tmp/a folder"}))
                .is_ok()
        );
        assert!(
            serde_json::from_value::<FreeSpaceResult>(
                serde_json::json!({"writable":false,"free_bytes":-1})
            )
            .is_err()
        );
    }
}
