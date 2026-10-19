#!/usr/bin/env python3
"""Fixture reducer plus actual pixel control for the Pieces bar."""
import os
import datetime
import shutil
from pathlib import Path
import subprocess
import sys
from PIL import Image
ROOT=Path(__file__).resolve().parents[2]
sys.path.insert(0,str(ROOT/'agent_ergonomics_audit/audit/regression_tests'))
from fbtlog import Case,suite_status
archive=ROOT/'target/test-logs/fbt-p6-02-bw1/runs'/datetime.datetime.now().strftime('%H%M%S')
for name in ['qs_pieces','qs_pieces_capture']:
    previous=ROOT/'target/test-logs'/name
    if previous.exists():shutil.copytree(previous,archive/name)
path=ROOT/'target/test-logs/fbt-p6-02-bw1/pieces-fixture.png'
path.parent.mkdir(parents=True,exist_ok=True)
# Preserve a failed render instead of reusing its bytes if capture fails.
if path.exists():path.rename(path.with_name('pieces-fixture-previous.png'))
result=subprocess.run([sys.executable,str(Path(__file__).with_name('run-qs.py')),'pieces',str(Path(__file__).with_name('pieces'))],env=dict(os.environ,FBT_PIECES_CAPTURE=str(path)))
with Case('qs_pieces_capture','visible_pixels') as case:
    if path.exists():
        image=Image.open(path).convert('RGB')
        blank=Image.new('RGB',image.size)
        case.check(not any(max(pixel)>64 for pixel in blank.get_flattened_data()),'blank capture negative control is rejected','zero visible foreground pixels')
        bright=sum(1 for pixel in image.get_flattened_data() if max(pixel)>64)
        case.check(result.returncode==0 and bright>500,'captured component contains visible bar/text pixels, not a blank item',f'exit={result.returncode}; size={image.size}; bright={bright}; {path}')
    else:case.check(False,'capture exists','no image')
raise SystemExit(result.returncode or suite_status('qs_pieces_capture'))
