#!/usr/bin/env python3
"""Loopback-only independent oracle. Uses only generated disposable data."""
import hashlib,json,pathlib,select,sys,time
import libtorrent as lt
role,encryption,directory=sys.argv[1:4]
directory=pathlib.Path(directory)
payload=bytes((i*31+i//97)%256 for i in range(192*1024+113))
if role=='seed':
    (directory/'payload.bin').write_bytes(payload)
    info={b'name':b'payload.bin',b'length':len(payload),b'piece length':16384,b'pieces':b''.join(hashlib.sha1(payload[i:i+16384]).digest() for i in range(0,len(payload),16384))}
    torrent=lt.bencode({b'info':info})
    (directory/'fixture.torrent').write_bytes(torrent)
    other=dict(info);other[b'name']=b'distractor.bin'
    (directory/'other.torrent').write_bytes(lt.bencode({b'info':other}))
    (directory/'distractor.bin').write_bytes(payload)
else:
    torrent=pathlib.Path(sys.argv[5]).read_bytes()
policy={'forced':lt.enc_policy.pe_forced,'disabled':lt.enc_policy.pe_disabled,'enabled':lt.enc_policy.pe_enabled}[encryption]
settings={'listen_interfaces':'127.0.0.1:0','enable_dht':False,'enable_lsd':False,'enable_upnp':False,'enable_natpmp':False,'enable_outgoing_utp':False,'enable_incoming_utp':False,'out_enc_policy':int(policy),'in_enc_policy':int(policy),'allowed_enc_level':2,'prefer_rc4':True,'allow_multiple_connections_per_ip':True,'download_rate_limit':2*1024*1024,'upload_rate_limit':2*1024*1024}
session=lt.session(settings)
tinfo=lt.torrent_info(torrent)
params=lt.parse_magnet_uri('magnet:?xt=urn:btih:'+str(tinfo.info_hash())) if role=='magnet-leech' else lt.add_torrent_params()
if role!='magnet-leech':params.ti=tinfo
params.save_path=str(directory);params.flags &= ~lt.torrent_flags.auto_managed;params.flags &= ~lt.torrent_flags.paused
handle=session.add_torrent(params)
if role=='seed':
    deadline=time.monotonic()+10
    while not handle.status().is_seeding:
        if time.monotonic()>deadline: raise RuntimeError('seed did not finish checking')
        time.sleep(.02)
else:
    address,port=sys.argv[4].rsplit(':',1);handle.connect_peer((address,int(port)))
seen_rc4=False;seen_plain=False;seen_peers=0
print(json.dumps({'addr':'127.0.0.1:'+str(session.listen_port()),'hash':str(tinfo.info_hash()),'version':lt.__version__}),flush=True)
while True:
    for peer in handle.get_peer_info():
        if peer.flags & (lt.peer_info.handshake | lt.peer_info.connecting): continue
        seen_peers+=1
        if peer.flags & lt.peer_info.rc4_encrypted: seen_rc4=True
        else: seen_plain=True
    ready,_,_=select.select([sys.stdin],[],[],.01)
    if ready:
        if not sys.stdin.readline():break
        status=handle.status()
        print(json.dumps({'rc4':seen_rc4,'plaintext':seen_plain,'peer_observations':seen_peers,'is_seeding':status.is_seeding,'has_metadata':status.has_metadata,'downloaded':status.total_payload_download,'uploaded':status.total_payload_upload}),flush=True)
