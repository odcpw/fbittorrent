"""Token masking for QML structure checks, preserving line numbers."""
import re
TOKENS=re.compile(r'//[^\n]*|/\*[\s\S]*?\*/|"(?:\\.|[^"\\])*"|\'(?:\\.|[^\'\\])*\'')
def cleaned(source):return TOKENS.sub(lambda match:'\n'*match.group().count('\n'),source)
