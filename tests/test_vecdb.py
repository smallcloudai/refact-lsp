import json
import requests

from typing import List, Dict, Any
from termcolor import colored


# TODO: check if file:10-20 is in results as well


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


def file_name_score(file_name: str, results: List[Dict]) -> int:
    score = -1
    for idx, r in enumerate(results, 1):
        if r.get('file_name', '').endswith(file_name):
            score = idx
            break
    return score


def test1() -> int:
    query = "@workspace usages of ast_based_file_splitter"
    results = at_preview_post(query)
    return file_name_score("ast_based_file_splitter.rs", results)


def test2() -> int:
    query = "@workspace what compiled in commands are there in toolbox?"
    results = at_preview_post(query)
    return file_name_score("toolbox_compiled_in.rs", results)


def test3() -> int:
    query = "@workspace fields in fields in code assistant caps"
    results = at_preview_post(query)
    return file_name_score("src/caps.rs", results)


def main():
    tests = [test1, test2, test3]
    for i, test in enumerate(tests, 1):
        score = test()
        result = "passed" if score else "failed"
        color = "green" if result == "passed" else "red"
        print(colored(f"test {i} {result}; score={score}", color))


if __name__ == "__main__":
    main()
