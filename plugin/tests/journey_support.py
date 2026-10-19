"""Real dashboard input in a private compositor; keeps its virtual seat attached."""
import subprocess
from dashboard import Session

class JourneySession(Session):
    def __init__(self,case,box):
        super().__init__(case,box)
        # A headless seat has no hardware keyboard. Keep one native virtual
        # device alive between commands, as the Shows journey does, so Qt does
        # not lose its active focus on every wtype process exit.
        self.keyboard=subprocess.Popen(['wtype','-s','600000'],env=self.env)
    def keys(self,args,predicate):
        self.compositor.ipc('[title="^fbittorrent$"] focus')
        try:return super().keys(args,predicate)
        except TimeoutError as error:raise TimeoutError('Journey keys '+repr(args)+': '+str(error)) from error
    def close(self):
        try:
            self.keyboard.terminate();self.keyboard.wait(timeout=3)
        finally:super().close()
