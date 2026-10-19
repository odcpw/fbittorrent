#!/usr/bin/env python3
"""Opt-in Arch headless tools in ignored test storage, without a system install."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import urllib.parse
import urllib.request

ROOT=Path(__file__).resolve().parents[2]/'plugin/tests/target/headless-tools'


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--download',action='store_true',help='download package URLs and SHA-256 values from the local pacman sync database')
    args=parser.parse_args()
    rows=subprocess.check_output(['pacman','-Sp','--print-format','%n %h %l','sway','wlroots0.20'],text=True).splitlines()
    if len(rows)>128:raise RuntimeError('unexpected package count')
    packages=[]
    for row in rows:
        name,digest,url=row.split();packages.append(dict(name=name,sha256=digest,url=url))
    print(json.dumps(packages,indent=2))
    if not args.download:return 0
    ROOT.mkdir(parents=True,exist_ok=True)
    for package in packages:
        url,digest=package['url'],package['sha256']
        parsed=urllib.parse.urlparse(url)
        if parsed.scheme!='https' or len(digest)!=64:raise RuntimeError('package requires HTTPS and SHA-256 metadata')
        path=ROOT/Path(parsed.path).name
        if not path.exists() or hashlib.sha256(path.read_bytes()).hexdigest()!=digest:
            with urllib.request.urlopen(url,timeout=30) as response,path.open('wb') as out:
                total=0
                while chunk:=response.read(65536):
                    total+=len(chunk)
                    if total>100*1024*1024:raise RuntimeError('package exceeds 100 MiB')
                    out.write(chunk)
        if hashlib.sha256(path.read_bytes()).hexdigest()!=digest:raise RuntimeError('package digest mismatch: '+path.name)
        subprocess.run(['bsdtar','-xf',str(path),'-C',str(ROOT)],check=True)
    (ROOT/'packages.json').write_text(json.dumps(packages,indent=2)+'\n')
    print('FBT_TEST_SWAY='+str(ROOT/'usr/bin/sway'))
    return 0

if __name__=='__main__':raise SystemExit(main())
