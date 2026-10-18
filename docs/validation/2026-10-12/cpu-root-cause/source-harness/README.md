# Public epoch-domain validation harness

This is a standalone consumer of vendored Asupersync. It enables
`native-runtime` and the explicitly diagnostic `epoch-diagnostics` feature,
with default features disabled. Asupersync is an ordinary dependency and is
never compiled with its internal `cfg(test)` modules or `test-internals`.

Run from any directory:

```bash
python3 /home/oliver/Projects/odcpw/fbittorrent/docs/validation/2026-10-12/cpu-root-cause/source-harness/run.py review
```

The wrapper selects Rust 1.98.0, offline locked resolution, two build jobs and
one test thread. Each launch creates a distinct log and receipt in the parent
directory. The initial receipt records running status and a source freeze;
completion records exit code, completion time, terminal test counts and log
SHA. An interrupted running receipt is never a pass. Source hashes must match
before and after, and all eight named tests must pass without filtering or
ignoring before `admissible_pass` can be true. Compiled artifacts live under
`target/cpu-root-cause-20261012/epoch-public-reconstruction` and can be rebuilt.

The reconstructed tests are new source. They are not the lost six-test file
identified by SHA `6cc447cb…`; the original incomplete `epoch-domain-tests.log`
remains separate. See [executed proof](../epoch-domain-public-proof.md).

This harness gates root's acceptance of the authorized independent-domain
correction after the measured 163,899 false ModuleDesync observations. Retire
this local harness after equivalent public regressions are retained upstream
and the correction is accepted. Its results establish bounded public API
semantics and native cleanup, with no CPU, memory or app qualification claim.
