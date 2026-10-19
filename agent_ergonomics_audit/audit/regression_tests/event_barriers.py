"""Wait for post-response owner samples instead of guessing scheduler latency."""
import json
import time


def summary_samples(client, count=2, timeout=8):
    sub=client.call('subscribe',dict(topics=['summary'],interval_ms=250))['result']['sub']
    samples=[];snapshot=False;deadline=time.monotonic()+timeout
    old=client.socket.gettimeout();client.socket.settimeout(timeout)
    try:
        for frame in client.events():
            if frame.get('sub')==sub:
                if frame.get('ev') in ('snapshot.end','state') and not snapshot:snapshot=True
                elif snapshot and frame.get('ev')=='state' and frame.get('topic')=='summary':
                    samples.append(frame['data'])
                    if len(samples)==count:return samples
            if time.monotonic()>deadline:raise TimeoutError('post-request summary samples: '+json.dumps(samples))
        raise EOFError('owner ended before post-request sample barrier')
    finally:
        client.socket.settimeout(old)
        # A bounded auxiliary subscription must not accumulate across assertions.
        client.call('unsubscribe',dict(sub=sub))
