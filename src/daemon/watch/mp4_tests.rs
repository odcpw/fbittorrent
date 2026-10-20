use super::*;
use std::{cell::Cell, io::Write};

fn atom(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
    [
        ((data.len() + 8) as u32).to_be_bytes().as_slice(),
        kind,
        data,
    ]
    .concat()
}
fn movie() -> Vec<u8> {
    let mut header = [0; 20];
    header[12..16].copy_from_slice(&1000u32.to_be_bytes());
    header[16..20].copy_from_slice(&64000u32.to_be_bytes());
    atom(b"moov", &atom(b"mvhd", &header))
}
fn inspect(bytes: &[u8], run: impl FnOnce(&Path)) {
    let root = std::env::temp_dir().join(format!(
        "fbt-mp4-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("movie.mp4");
    std::fs::File::create(&path)
        .unwrap()
        .write_all(bytes)
        .unwrap();
    run(&path);
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn faststart_and_tail_moov_need_the_entire_verified_box() {
    let media = atom(b"mdat", &[7; 4096]);
    for fast in [true, false] {
        let moov = movie();
        let start = if fast { 0 } else { media.len() as u64 };
        let end = start + moov.len() as u64;
        let bytes = if fast {
            [moov, media.clone()].concat()
        } else {
            [media.clone(), moov].concat()
        };
        inspect(&bytes, |path| {
            let mut parsed = Movie::default();
            assert!(!parsed.inspect(path, bytes.len() as u64, |r| !(r.start <= start + 16 && r.end > start + 16)), "a hole inside moov must block readiness even with its header and last bytes verified");
            assert!(parsed.inspect(path, bytes.len() as u64, |_| true));
            assert_eq!(parsed.duration(), Some(64.));
            assert!(
                !parsed.inspect(path, bytes.len() as u64, |r| r.end <= start
                    || r.start >= end),
                "cached layout never bypasses the latest have snapshot"
            );
        });
    }
}
#[test]
fn follows_sizes_instead_of_scanning_payload_and_bounds_hostile_layouts() {
    for bytes in [
        atom(b"mdat", &movie()), // A moov-shaped byte string inside media is not a box.
        atom(b"moov", &[]),
        [
            1u32.to_be_bytes().as_slice(),
            b"mdat",
            &u64::MAX.to_be_bytes(),
        ]
        .concat(),
        [4u32.to_be_bytes().as_slice(), b"moov"].concat(),
        [999u32.to_be_bytes().as_slice(), b"moov"].concat(),
        [atom(b"free", &[]).repeat(HEADERS + 1), movie()].concat(),
    ] {
        inspect(&bytes, |path| {
            let probes = Cell::new(0);
            assert!(!Movie::default().inspect(path, bytes.len() as u64, |_| {
                probes.set(probes.get() + 1);
                true
            }));
            assert!(probes.get() <= HEADERS * 2 + 1);
        });
    }
}
#[test]
fn accepts_extended_and_end_of_file_box_sizes() {
    let regular = movie();
    for bytes in [
        [
            1u32.to_be_bytes().as_slice(),
            b"moov",
            &(regular.len() as u64 + 8).to_be_bytes(),
            &regular[8..],
        ]
        .concat(),
        [0u32.to_be_bytes().as_slice(), &regular[4..]].concat(),
    ] {
        inspect(&bytes, |path| {
            let mut movie = Movie::default();
            assert!(movie.inspect(path, bytes.len() as u64, |_| true));
            assert_eq!(movie.duration(), Some(64.));
        });
    }
}
#[test]
fn verified_ranges_clip_shared_pieces_and_reject_overflow() {
    use crate::daemon::watch::{iso_movie, verified_range};
    assert!(verified_range(10, 4, 0..6, |i| i == 2 || i == 3));
    assert!(!verified_range(10, 4, 0..7, |i| i == 2 || i == 3));
    assert!(!verified_range(u64::MAX, 4, 1..2, |_| true));
    assert!(!verified_range(0, 0, 0..1, |_| true));
    for extension in ["mp4", "MOV", "M4v"] {
        assert!(iso_movie(Path::new(&format!("movie.{extension}"))));
    }
    assert!(!iso_movie(Path::new("movie.mkv")));
}
