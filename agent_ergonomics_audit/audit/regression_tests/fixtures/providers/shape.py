"""Local stable CLI characterization; no network access."""
class shape:
    name = 'Shape provider'
    url = 'https://shape.invalid'
    def search(self, query):
        from novaprinter import prettyPrinter
        if query == 'empty':
            import signal
            assert signal.getsignal(signal.SIGTERM) == signal.SIG_DFL, 'pool worker must not inherit the Nova group-killing handler'
            return
        if query == 'fail': raise RuntimeError('local provider failure')
        for digit,name,size,seeds in [('1','Sintel.2010.1080p.mkv',4096,9),('2','Big.Buck.Bunny.720p.mp4',8192,7),('3','Linux.iso',1024,0)]:
            prettyPrinter(dict(link='magnet:?xt=urn:btih:'+digit*40,name=name,size=size,seeds=seeds,leech=1,engine_url=self.url))
