"""Independent disk/metainfo facts, not an invocation of the CLI's serializer."""

def assert_fixture_rows(rows, item, downloads):
    expected=dict(hash=item.info_hash,name=item.info['name'],
                  size=sum(path.stat().st_size for path in item.files),save_path=str(downloads))
    assert len(rows)==1, f'one owned fixture expected, got {rows!r}'
    actual={key:rows[0].get(key) for key in expected}
    assert actual==expected, f'owner facts differ: expected={expected!r}, actual={actual!r}'
    return expected
