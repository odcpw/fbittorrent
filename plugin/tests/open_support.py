"""Allow only the production no-start Open CLI through a private recording shim."""
import shlex
import json
import fixtures


def allow_open(box, binary):
    fixtures.path_shim('fbittorrent',
                       'if [ "$1" = "--no-start" ] && [ "$2" = "--json" ] && [ "$3" = "open" ]; then\n'
                       '  exec '+shlex.quote(str(binary))+' "$@"\n'
                       'fi\nexit 99', box.root)


def calls(box, name):
    path=box.root/'shims'/(name+'.log')
    return [json.loads(line) for line in path.read_text().splitlines()] if path.exists() else []


def launcher_calls(box):
    return {name:calls(box,name) for name in ('xdg-open','mpv','ffplay','vlc')}


def blocked_payload_error(message):
    text=message.lower()
    return 'refusing to open' in text or 'file is incomplete and cannot be streamed' in text


def add_text_torrent(box, client):
    """Add a real, already-present safe document, separate from blocked .bin fixtures."""
    source=box.root/'safe-open-source';source.mkdir()
    payload=source/'Open fixture.txt'
    payload.write_text('Private document for literal file-open checks.\n')
    item=fixtures.torrent_fixture(source,[payload],name='safe-open')
    client.call('torrents.add',dict(source=dict(path=str(item.torrent)),save_path=str(source)))
    return item,payload
