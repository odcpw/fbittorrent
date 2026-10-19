# CLI characterization

`search-212c3cd.json` and `.txt` were captured from the real pre-P3-07 native
CLI, using the resolved source committed as `212c3cd` and the local `shape.py`
Nova provider in a retaining Sandbox. The JSON and human stdout were copied
verbatim before the new CLI routing was built. Evidence:
`target/test-logs/fbt-p3-07-c2p/legacy-capture/agent_contract/search_legacy_capture.jsonl`.
The binary's package version is 0.1.0; no v0.1.2 tag/binary was available in this
checkout, so these files do not claim that provenance.

The new result's only added field is `providers[]`; the characterization
comparison removes that planned addition and compares every existing byte.
Both live source paths must also compare directly byte-for-byte, after the
contract's optional `warnings[]` removal. Do not regenerate these files from the
new implementation.
