import json
import requests

from typing import List, Dict, Any
from termcolor import colored


def at_preview_post(query: str) -> List[Dict[str, Any]]:
    payload = {
        "query": query,
        "model": "gpt-4",
    }
    response = requests.post(
        "http://localhost:8001/v1/at-command-preview",
        data=json.dumps(payload),
    )
    assert response.status_code == 200, f"Response code: {response.status_code}: {response.text}"

    decoded = json.loads(json.loads(response.text)['messages'][0]['content'])

    return decoded


def test1() -> int:
    query = "@workspace usages of ast_based_file_splitter"
    results = at_preview_post(query)
    if any("vectorizer_service.rs" in r['file_name'] for r in results):
        return 1
    else:
        print([r['file_name'] for r in results])
        return 0


def test2() -> int:
    query = "@workspace what compiled in commands are there in toolbox?"
    results = at_preview_post(query)
    if any("toolbox_compiled_in.rs" in r['file_name'] for r in results):
        return 1
    else:
        print([r['file_name'] for r in results])
    return 0


def test3() -> int:
    query = "@workspace fields in fields in code assistant caps"
    results = at_preview_post(query)
    if any("src/caps.rs" in r['file_name'] for r in results):
        return 1
    else:
        print([r['file_name'] for r in results])
    return 0


def main():
    tests = [test1, test2, test3]
    for i, test in enumerate(tests, 1):
        result = "passed" if test() else "failed"
        color = "green" if result == "passed" else "red"
        print(colored(f"test {i} {result}", color))


if __name__ == "__main__":
    main()
