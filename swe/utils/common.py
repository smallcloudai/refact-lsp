import whatthepatch

from pathlib import Path


def patched_file(patch: str) -> str:
    files = list(whatthepatch.parse_patch(patch))
    assert len(files) == 1
    header = files[0].header
    filename = header.old_path
    if header.old_path != header.new_path:
        assert header.old_path[len("a/"):] == header.new_path[len("b/"):]
        filename = header.old_path[len("a/"):]
    return filename


def filename_mentioned(filename: str, text: str) -> str:
    if filename in text:
        return "fully"
    elif Path(filename).name in text:
        return "partially"
    return "no"
