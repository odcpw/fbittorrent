"""Local Nova fixture: the better-seeded copy of Alpha's damaged title."""
class beta:
    name = 'Local Beta'
    url = 'https://beta.invalid'
    def search(self, query):
        from novaprinter import prettyPrinter
        prettyPrinter(dict(link='magnet:?xt=urn:btih:'+'a'*40+'&dn=other', name='è¾›ç‰¹å°”',
                          size=1234, seeds=9, leech=-1, engine_url=self.url))
