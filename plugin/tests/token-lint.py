#!/usr/bin/env python3
"""Reject styling literals and unapproved collection polling in production QML/JS."""
import json
from pathlib import Path
import re
import sys

ROOT=Path(__file__).resolve().parents[2]
COMMENTS=re.compile(r'//[^\n]*|/\*[\s\S]*?\*/|"(?:\\.|[^"\\])*"|\'(?:\\.|[^\'\\])*\'')
NAMES=set('''aliceblue antiquewhite aqua aquamarine azure beige bisque black blanchedalmond blue blueviolet brown burlywood cadetblue chartreuse chocolate coral cornflowerblue cornsilk crimson cyan darkblue darkcyan darkgoldenrod darkgray darkgreen darkgrey darkkhaki darkmagenta darkolivegreen darkorange darkorchid darkred darksalmon darkseagreen darkslateblue darkslategray darkslategrey darkturquoise darkviolet deeppink deepskyblue dimgray dimgrey dodgerblue firebrick floralwhite forestgreen fuchsia gainsboro ghostwhite gold goldenrod gray green greenyellow grey honeydew hotpink indianred indigo ivory khaki lavender lavenderblush lawngreen lemonchiffon lightblue lightcoral lightcyan lightgoldenrodyellow lightgray lightgreen lightgrey lightpink lightsalmon lightseagreen lightskyblue lightslategray lightslategrey lightsteelblue lightyellow lime limegreen linen magenta maroon mediumaquamarine mediumblue mediumorchid mediumpurple mediumseagreen mediumslateblue mediumspringgreen mediumturquoise mediumvioletred midnightblue mintcream mistyrose moccasin navajowhite navy oldlace olive olivedrab orange orangered orchid palegoldenrod palegreen paleturquoise palevioletred papayawhip peachpuff peru pink plum powderblue purple rebeccapurple red rosybrown royalblue saddlebrown salmon sandybrown seagreen seashell sienna silver skyblue slateblue slategray slategrey snow springgreen steelblue tan teal thistle tomato transparent turquoise violet wheat white whitesmoke yellow yellowgreen'''.split())
HEX=re.compile(r'#[0-9a-fA-F]{3,8}\b')
NUMBER=r'(?:0[xX][0-9a-fA-F]+|[+-]?(?:\d+(?:\.\d*)?|\.\d+)(?:[eE][+-]?\d+)?)'
RGBA=re.compile(r'\bQt\.rgba\s*\(\s*'+NUMBER+r'\s*,\s*'+NUMBER+r'\s*,\s*'+NUMBER)
NAMED=re.compile(r'["\']([a-z]+)["\']',re.I)
SIZE=re.compile(r'(?<![\w.])(?:width|height|implicitWidth|implicitHeight|spacing|margins|pixelSize|font\.pixelSize|anchors\.(?:margins|leftMargin|rightMargin|topMargin|bottomMargin))\s*:\s*[+-]?(?:\d+(?:\.\d*)?|\.\d+)\b')
DURATION=re.compile(r'\bduration\s*:\s*(?!\s*(?:Commons\.)?Style\.duration\s*\()\S')

def violations(text):
    clean=COMMENTS.sub(lambda m:'\n'*m.group().count('\n') if m.group().startswith(('//','/*')) else m.group(),text)
    found=[]
    for number,line in enumerate(clean.splitlines(),1):
        if HEX.search(line):found.append((number,'hex_color'))
        if RGBA.search(line):found.append((number,'rgba_color'))
        if any(m.group(1).lower() in NAMES for m in NAMED.finditer(line)):found.append((number,'named_color'))
        if SIZE.search(line):found.append((number,'pixel_size'))
        if DURATION.search(line):found.append((number,'duration'))
        for op in ('torrents.list','torrents.files'):
            if op in line:found.append((number,op))
    return found

def allowlist():
    path=Path(__file__).with_name('lint-allow.txt')
    items=[]
    for line in path.read_text().splitlines():
        if not line.strip() or line.startswith('#'):continue
        item=json.loads(line)
        if not all(item.get(key) for key in ('rule','path','source','reason')):raise ValueError('every exception needs an exact source and reason')
        items.append(item)
    return items

def check(root):
    allowed=allowlist();used=set();errors=[]
    for path in sorted(root.rglob('*')):
        if path.suffix.lower() not in ('.qml','.js'):continue
        source=path.read_text();lines=source.splitlines();relative=str(path.relative_to(root))
        for number,rule in violations(source):
            matches=[i for i,item in enumerate(allowed) if item['path']==relative and item['rule']==rule and item['source']==lines[number-1].strip()]
            if len(matches)==1 and matches[0] not in used:used.add(matches[0])
            else:errors.append(f'{relative}:{number}: {rule}: {lines[number-1].strip()}')
    for i,item in enumerate(allowed):
        if i not in used:errors.append(f'stale lint exception: {item["path"]}: {item["rule"]}')
    return errors

def self_test():
    sys.path.insert(0,str(ROOT/'agent_ergonomics_audit/audit/regression_tests'))
    from fbtlog import Case,suite_status
    with Case('qs_token_lint','negative_controls') as case:
        planted=Path(__file__).with_name('lint-negative.qml').read_text()
        expected=['hex_color','rgba_color','named_color','pixel_size','duration','torrents.list','torrents.files']
        observed=[rule for _,rule in violations(planted)]
        case.check(sorted(observed)==sorted(expected),'negative fixture fails each rule exactly once',json.dumps(observed))
        for rule in expected:case.check(observed.count(rule)==1,'planted '+rule+' is rejected once',json.dumps(observed))
        case.check(not violations('property real extent: parent ? parent.width : 0'),'member reads in ternaries are not pixel declarations',str(violations('property real extent: parent ? parent.width : 0')))
        good='Rectangle { width: Style.space(380); height: Commons.Style.spacing.controlHeight; color: Commons.Color.foreground; NumberAnimation { duration: Style.duration(400) } }'
        case.check(not violations(good),'shell tokens and wrapped duration pass',str(violations(good)))
        case.check(not violations('// color: "#ff0000"\nText {text:"https://example.org"}'),'comments are inert and URL strings stay intact',str(violations('// color: "#ff0000"')))
        case.check(violations('Text { color: "rebeccapurple" }')==[(1,'named_color')],'named-color rule covers CSS names beyond basic RGB','rebeccapurple rejected')
        case.check(violations('Text { width: 10; color: "#fff" }')==[(1,'hex_color'),(1,'pixel_size')],'one allowed rule cannot mask another violation on the same line','two independent findings')
    return suite_status('qs_token_lint')

if __name__=='__main__':
    result=self_test()
    errors=check(Path(sys.argv[1]))
    if errors:print('\n'.join(errors),file=sys.stderr)
    sys.exit(result or bool(errors))
