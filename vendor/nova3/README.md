# Nova search runtime

These helper scripts are from qBittorrent Nova 1.53 (installed by qBittorrent
on this machine). Their original copyright and BSD licence notices are
retained in each file. Upstream: https://github.com/qbittorrent/qBittorrent/tree/master/src/searchengine/nova3

fbittorrent changes to nova2.py cap concurrent search workers at four and add
Linux parent-death cleanup to each worker. Installed search providers retain
their own licences and are copied into the user data directory separately.

## Licences

Checked file by file on 2026-10-18 (plan item P0-03). Every file carries the
3-clause BSD licence text in its header, which permits embedding in the MIT
fbittorrent binary (`include_str!` in `src/search.rs`) as long as the notices
travel with it; the files are embedded and written out verbatim, headers
included, and the repository `NOTICE` names them.

| File | Licence | Copyright line |
|---|---|---|
| `helpers.py` | BSD-3-Clause (header) | none; "Author: Christophe DUMEZ (chris@qbittorrent.org)" |
| `nova2.py` | BSD-3-Clause (header, "Licence: BSD") | none; "Author: Fabien Devaux", contributors Christophe Dumez and others |
| `nova2dl.py` | BSD-3-Clause (header) | none; "Author: Christophe DUMEZ", contributor Vladimir Golovnev |
| `novaprinter.py` | BSD-3-Clause (header) | none (no author line) |
| `socks.py` | BSD-3-Clause (header, PySocks 1.7.1) | "Copyright 2006 Dan-Haim. All rights reserved." |
| `README.md` | this note (fbittorrent, MIT) | n/a |
