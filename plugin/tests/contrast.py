#!/usr/bin/env python3
"""Shared 8-bit sRGB dim ladder oracle; fixture-only by default, explicit refresh."""
import argparse
import json
import math
from pathlib import Path
import shutil
import sys
import tomllib
sys.path.insert(0,str(Path(__file__).resolve().parents[2]/'agent_ergonomics_audit/audit/regression_tests'))
from fbtlog import Case, suite_status

ROOT=Path(__file__).resolve().parents[2]
THEMES=ROOT/'contracts/ui/themes'


def rgb(value): return tuple(int(value[i:i+2],16) for i in (1,3,5))
def color(values): return '#'+''.join(f'{max(0,min(255,math.floor(v+.5))):02x}' for v in values)
def lum(value):
    v=[c/255 for c in rgb(value)]
    v=[c/12.92 if c<=.04045 else ((c+.055)/1.055)**2.4 for c in v]
    return sum(c*w for c,w in zip(v,(.2126,.7152,.0722)))
def ratio(a,b):
    a,b=sorted((lum(a),lum(b)))
    return (b+.05)/(a+.05)
def mix(a,b,w): return color(w*x+(1-w)*y for x,y in zip(rgb(a),rgb(b)))
def reference(theme):
    fg,bg,accent=(theme[k] for k in ('foreground','background','accent'))
    rungs=[color(c/1.4 for c in rgb(fg)),mix(fg,bg,.75),mix(fg,bg,.85),fg]
    rung=next((i for i,v in enumerate(rungs,1) if ratio(v,bg)>=4.5),4)
    dim=rungs[rung-1]
    return dict(foreground=fg,background=bg,accent=accent,dim=dim,dim_rung=rung,
                dim_ratio=round(ratio(dim,bg),6),fg_ratio=round(ratio(fg,bg),6),
                accent_ratio=round(ratio(accent,bg),6),accent_text=ratio(accent,bg)>=4.5,
                low_chroma=max(rgb(accent))-min(rgb(accent))<20,
                muted_ratio=round(ratio(theme['muted'],bg),6) if 'muted' in theme else None)

def installed():
    result={p.parent.name:p for p in Path('/usr/share/omarchy/themes').glob('*/colors.toml')}
    owner=Path.home()/'.local/state/omarchy/current/theme/colors.toml'
    if owner.exists(): result['owner']=owner
    return result

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--refresh',action='store_true',help='copy installed and owner theme files; regenerate the checked-in oracle')
    parser.add_argument('--installed',action='store_true',help='also check currently installed colors without writing fixtures')
    parser.add_argument('--self-test',action='store_true')
    args=parser.parse_args()
    THEMES.mkdir(parents=True,exist_ok=True)
    if args.refresh:
        for name,path in installed().items(): shutil.copyfile(path,THEMES/(name+'.toml'))
        rows={p.stem:reference(tomllib.loads(p.read_text())) for p in sorted(THEMES.glob('*.toml')) if not p.name.startswith('_')}
        (THEMES/'expected.json').write_text(json.dumps(rows,indent=2)+'\n')
    expected=json.loads((THEMES/'expected.json').read_text())
    with Case('qs_contrast','themes') as case:
        for path in sorted(THEMES.glob('*.toml')):
            if path.name.startswith('_'): continue
            actual=reference(tomllib.loads(path.read_text()))
            case.check(actual==expected.get(path.stem),f'{path.stem} agrees with the shared dim oracle',json.dumps(actual))
            case.check(actual['fg_ratio']>=4.5 and actual['dim_ratio']>=4.5,f'{path.stem} foreground and computed dim reach 4.5:1',json.dumps(actual))
        case.check(set(expected)=={p.stem for p in THEMES.glob('*.toml') if not p.name.startswith('_')},'every fixture theme is checked','literal file/expected keys compared')
        if args.installed:
            for name,path in sorted(installed().items()):
                actual=reference(tomllib.loads(path.read_text()))
                case.check(actual['fg_ratio']>=4.5 and actual['dim_ratio']>=4.5,f'installed {name} passes',json.dumps(actual))
        if args.self_test:
            bad=reference(tomllib.loads((THEMES/'_bad.toml').read_text()))
            case.check(bad['fg_ratio']<4.5 and bad['dim_ratio']<4.5,'known-bad theme fails the contrast gate',json.dumps(bad))
            case.check(ratio('#000000','#ffffff')==21 and ratio('#ffffff','#ffffff')==1,'WCAG known endpoints','black/white=21; identical=1')
    return suite_status('qs_contrast')

if __name__=='__main__':sys.exit(main())
