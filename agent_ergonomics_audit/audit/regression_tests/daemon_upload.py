"""Owner upload policy: actual piece requests, default refusal and explicit permission."""
import base64
import json
import tomllib
from pathlib import Path
from upload_wire import incoming, Outbound
import daemon as d
import fixtures


def verified_piece(pair):
    pieces=pair.client.call('torrents.pieces',dict(hash=pair.hash))['result']
    assert pieces['piece_size']==16384,pieces
    for byte,mask in enumerate(base64.b64decode(pieces['have'],validate=True)):
        for bit in range(8):
            if mask&(128>>bit):return byte*8+bit
    raise AssertionError('payload probe needs a real verified piece')


def payload_request(pair):
    port=tomllib.loads((Path(pair.box.env['XDG_CONFIG_HOME'])/'fbittorrent/config.toml').read_text())['peer_port']
    proof=incoming(('127.0.0.1',port),pair.hash,pair.payload,piece=verified_piece(pair))
    pair.case.check(proof.handshake and proof.request_sent,
        'real incoming handshake and verified-piece request completed',json.dumps(proof.describe()))
    return proof.payload_bytes


def outbound_request(pair,leecher,expected):
    leecher.connected()
    rows=pair.client.call('torrents.peers',dict(hash=pair.hash))['result']['peers']
    pair.case.check(any(row['addr']==f'{leecher.addr[0]}:{leecher.addr[1]}' and row['incoming'] is False for row in rows),
        'daemon dialled the advertised leecher from the partially downloaded torrent',json.dumps(rows))
    proof=leecher.request()
    pair.case.check(proof.handshake and proof.request_sent and proof.payload_bytes==expected,
        'outbound verified-piece request enforces the actual upload permission',json.dumps(proof.describe()))


def zero_upload(binary,systemd):
    with d.Case('daemon','zero_upload') as case, Outbound() as outgoing_off, Outbound() as outgoing_on, \
            fixtures.daemon_pair(case,binary,systemd,65536,extra_peers=[outgoing_off.addr,outgoing_on.addr]) as pair:
        c=pair.client
        def row():return c.call('torrents.get',dict(hash=pair.hash))['result']['torrent']
        initial=row()
        piece=verified_piece(pair)
        outgoing_off.configure(pair.hash,pair.payload,piece);outgoing_on.configure(pair.hash,pair.payload,piece)
        off=payload_request(pair)
        case.check(initial['upload_enabled'] is False and 'upload_limit_bps' not in initial and off==0,'default downloading torrent serves zero piece payload without an implicit rate cap',json.dumps(dict(row=initial,payload_bytes=off)))
        outbound_request(pair,outgoing_off,0)
        reply=pair.h.cli('settings','--upload','on')
        case.check(reply.get('upload')=='on' and reply.get('restart') is False,'CLI enables global upload live without reopening the engine',json.dumps(reply))
        downloading_on=payload_request(pair)
        case.check(downloading_on==16384,'global upload on serves a verified piece while downloading',str(downloading_on))
        outbound_request(pair,outgoing_on,16384)
        c.call('torrents.set_seed',dict(hashes=[pair.hash],on=False))
        case.check(payload_request(pair)==0 and row()['upload_enabled'] is False,'explicit torrent off overrides global on',json.dumps(row()))
        pair.h.cli('settings','--upload','off')
        # A rate-only change must not grant permission; the source opts in separately.
        c.call('torrents.set_upload_limit',dict(hashes=[pair.hash],bps=None))
        case.check(payload_request(pair)==0,'unlimited rate still serves zero payload when permission is off',json.dumps(row()))
        d.DaemonContract.throttle(pair.seed_client,pair.hash,None)
        assert pair.h.wait_for(lambda:((value:=row())['state']=='done',json.dumps(value)),'default-off client finishes downloading normally',timeout=20)
        case.check(row()['upload_enabled'] is False and row()['state']=='done',
                   'completed default-off torrent stops instead of admitting an upload handle',json.dumps(row()))
        c.call('torrents.set_seed',dict(hashes=[pair.hash],on=True))
        assert pair.h.wait_for(lambda:((value:=row())['state']=='seeding',json.dumps(value)),'explicit torrent upload on admits the completed payload')
        complete_on=payload_request(pair)
        case.check(complete_on==16384 and row()['upload_enabled'] is True,'completed explicit upload on serves requested payload with global upload off',json.dumps(dict(row=row(),payload_bytes=complete_on,downloading_on_bytes=downloading_on)))
        c.call('torrents.set_seed',dict(hashes=[pair.hash],on=False))
        case.check(row()['state']=='done' and row()['upload_enabled'] is False,
                   'turning completed torrent upload off returns it to stopped completion',json.dumps(row()))
