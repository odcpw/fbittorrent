# Nova search runtime

These helper scripts are from qBittorrent Nova 1.53 (installed by qBittorrent
on this machine). Their original copyright and BSD licence notices are
retained in each file. Upstream: https://github.com/qbittorrent/qBittorrent/tree/master/src/searchengine/nova3

fbittorrent changes to nova2.py cap concurrent search workers at four and add
Linux parent-death cleanup to each worker. helpers.py gives every socket a
20-second default timeout, passes it to `urlopen`, and bounds each response to
16 MiB read and 64 MiB after gunzip (the app also caps each Python process's
address space and each search's duration). Installed search providers retain
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

fbittorrent P3-07: spawned pool workers reset SIGTERM to SIG_DFL before provider
code runs. Python retires these workers with SIGTERM on normal pool exit; Nova's
parent alone retains the group-killing handler for cancellation and parent death.
Workers retain their parent-death SIGKILL. The local `shape` provider's empty
query asserts this signal policy, and CLI source-agreement/cancellation cases
exercise the actual multiprocessing pool.

The group-killing TERM handler is installed only when Nova runs as the main
script, never when spawn imports it as `__mp_main__`. A pool can retire an idle
worker before its initializer has run; the worker's import must therefore keep
the default TERM handler too. The importing-worker regression holds one worker
inside helper import while another finishes both providers and retires the pool.

With fbittorrent interface binding enabled, nova2.py and nova2dl.py enable the
daemon-provided SOCKS5h proxy before importing engines. helpers.py also routes
create_connection and getaddrinfo through remote DNS, refuses local DNS lookups
and UDP, and keeps the proxy enabled when a provider calls enable_socks_proxy(False).
The daemon owns authenticated CONNECT-only endpoints, bounds connections and
timeouts, and closes each endpoint and its relays with the provider runner.
