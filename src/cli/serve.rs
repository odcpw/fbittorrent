//! Avoid faulting the interactive CLI into the long-lived daemon's RSS.
use super::{Cli, Cmd};
use clap::Parser;
use std::ffi::OsString;

pub(crate) fn parse_entry<I, T>(args: I) -> Result<Cli, clap::Error>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString>,
{
    let args: Vec<OsString> = args.into_iter().map(Into::into).collect();
    if let Some(daemon) = daemon_entry(&args) {
        return Ok(daemon);
    }
    canonical(args)
}

// Recognize only the simple successful daemon entry. Help, diagnostics,
// unusual spellings and every interactive command use the canonical parser.
fn daemon_entry(args: &[OsString]) -> Option<Cli> {
    let mut args = args.iter().skip(1);
    let (mut serve, mut json, mut no_start, mut robot_triage, mut no_notify) = (false, false, false, false, false);
    let mut idle_exit_secs = None;
    while let Some(arg) = args.next() {
        match arg.to_str()? {
            "serve" if !serve => serve = true,
            "--json" if !json => json = true,
            "--no-start" if !no_start => no_start = true,
            "--robot-triage" if !serve && !robot_triage => robot_triage = true,
            "--no-notify" if serve && !no_notify => no_notify = true,
            "--idle-exit-secs" if serve && idle_exit_secs.is_none() => {
                idle_exit_secs = Some(seconds(args.next()?.to_str()?)?);
            }
            value if serve && idle_exit_secs.is_none() && value.starts_with("--idle-exit-secs=") => {
                idle_exit_secs = Some(seconds(value.strip_prefix("--idle-exit-secs=")?)?);
            }
            _ => return None,
        }
    }
    serve.then_some(Cli { json, no_start, robot_triage, cmd: Some(Cmd::Serve { idle_exit_secs, no_notify }) })
}

fn seconds(value: &str) -> Option<u64> {
    (!value.is_empty() && value.bytes().all(|b| b.is_ascii_digit())).then(|| value.parse().ok()).flatten()
}

#[cold]
#[inline(never)]
fn canonical(args: Vec<OsString>) -> Result<Cli, clap::Error> {
    Cli::try_parse_from(args)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn equivalent(args: Vec<&str>) {
        let argv: Vec<_> = std::iter::once("fbittorrent").chain(args).collect();
        let slim = parse_entry(argv.clone()).unwrap();
        let full = Cli::try_parse_from(argv).unwrap();
        assert_eq!((slim.json, slim.no_start, slim.robot_triage), (full.json, full.no_start, full.robot_triage));
        let (Some(Cmd::Serve { idle_exit_secs: left, no_notify: left_notify }), Some(Cmd::Serve { idle_exit_secs: right, no_notify: right_notify })) = (slim.cmd, full.cmd) else { panic!("serve entry changed the operation") };
        assert_eq!((left, left_notify), (right, right_notify));
    }

    #[test]
    fn daemon_entry_preserves_canonical_arguments() {
        for json in [false,true] {
            for no_start in [false,true] {
                for no_notify in [false,true] {
                    for seconds in [None,Some("0"),Some("300"),Some("18446744073709551615")] {
                        let mut args=vec!["serve"];
                        if json {args.push("--json");}
                        if no_start {args.push("--no-start");}
                        if no_notify {args.push("--no-notify");}
                        if let Some(value)=seconds {args.extend(["--idle-exit-secs",value]);}
                        equivalent(args);
                    }
                }
            }
        }
        equivalent(vec!["--json","--no-start","--robot-triage","serve","--idle-exit-secs=300"]);
    }

    #[test]
    fn help_errors_and_other_commands_use_the_canonical_parser() {
        for args in [
            vec!["serve", "--help"], vec!["help", "serve"],
            vec!["--version", "serve"], vec!["serve", "--unknown"],
            vec!["serve", "--idle-exit-secs", "-1"],
            vec!["serve", "--idle-exit-secs=18446744073709551616"],
            vec!["serve", "--idle-exit-secs"], vec!["serve", "--robot-triage"],
            vec!["serve", "--json", "--json"], vec!["--no-notify","serve"],
        ] {
            let argv: Vec<_> = std::iter::once("fbittorrent").chain(args).collect();
            let slim = parse_entry(argv.clone()).err().expect("argument should not start a daemon");
            let full = Cli::try_parse_from(argv).err().unwrap();
            assert_eq!(slim.kind(), full.kind());
            assert_eq!(slim.to_string(), full.to_string());
        }
        for args in [vec!["status"], vec!["show", "serve"], vec!["--json", "list"]] {
            let argv: Vec<_> = std::iter::once("fbittorrent").chain(args).collect();
            let slim = parse_entry(argv.clone()).unwrap();
            let full = Cli::try_parse_from(argv).unwrap();
            assert_eq!(std::mem::discriminant(&slim.cmd.unwrap()), std::mem::discriminant(&full.cmd.unwrap()));
            assert_eq!(slim.json, full.json);
        }
        let unusual=[OsString::from("fbittorrent"),OsString::from("serve"),OsString::from("--idle-exit-secs=+3")];
        assert!(daemon_entry(&unusual).is_none());
        equivalent(vec!["serve","--idle-exit-secs=+3"]);
    }
}
