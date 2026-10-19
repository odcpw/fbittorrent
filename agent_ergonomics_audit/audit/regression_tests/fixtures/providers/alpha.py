"""Local Nova fixture: duplicate, damaged UTF-8 and a cancellable process group."""
class alpha:
    name = 'Local Alpha'
    url = 'https://alpha.invalid'
    def search(self, query):
        import json, os, pathlib, subprocess, sys, time
        from novaprinter import prettyPrinter
        raw = 'è¾›ç‰¹å°”'
        pathlib.Path('alpha-raw.txt').write_bytes(raw.encode('utf-8'))
        prettyPrinter(dict(link='magnet:?xt=urn:btih:'+'a'*40, name=raw,
                          size=1234, seeds=2, leech=-1, engine_url=self.url))
        if query.startswith('hold'):
            child = subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(90)'])
            directory = pathlib.Path('fixture-groups'); directory.mkdir(exist_ok=True)
            (directory / (str(os.getpid())+'.json')).write_text(json.dumps(dict(pgid=os.getpgrp(),child=child.pid,query=query)))
            time.sleep(90)
