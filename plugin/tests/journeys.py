#!/usr/bin/env python3
"""The three M2 owner workflows, through a real dashboard in a private compositor."""
import argparse
from pathlib import Path
import sys

if __name__=='__main__':
    import private_net
    private_net.enter()
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary',type=Path,required=True)
    parser.add_argument('--workflow',choices=['W1','W2','W8','all'],default='all')
    args=parser.parse_args();binary=args.binary.resolve()
    from journey_cleanup import run as cleanup
    from journey_find import run as find_watch
    from journey_shows import run as follow_shows
    workflows={'W1':find_watch,'W2':follow_shows,'W8':cleanup}
    status=0
    for name,run in workflows.items():
        if args.workflow in ('all',name):status|=run(binary)
    sys.exit(status)
