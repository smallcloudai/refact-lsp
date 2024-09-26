import requests
import json
import pathlib
from termcolor import colored

# TODO: SecretaryBird


BASE_DIR = pathlib.Path(__file__).parent
FROG_PY = BASE_DIR / "emergency_frog_situation" / "frog.py"
TEST11_DATA = BASE_DIR / "test11_data"
TOAD_ORIG = BASE_DIR / "test11_data" / "toad_orig.py"


def patch_request(messages, ticket_ids):
    payload = {
        "messages": messages,
        "ticket_ids": ticket_ids,
    }
    resp = requests.post(
        "http://localhost:8001/v1/patch-single-file-from-ticket",
        data=json.dumps(payload),
    )
    assert resp.status_code == 200, resp.text
    return resp.json()


def make_messages(ticket_text: str):
    return [
        {"role": "assistant", "content": ticket_text}
    ]


def test01_rewrite_whole_file():
    text_expected = "# FROG"
    ticket_text = \
f"""📍REWRITE_WHOLE_FILE 001 {FROG_PY}
```python
{text_expected}
```
"""
    messages = make_messages(ticket_text)
    resp = patch_request(messages, ["001"])

    res0 = resp["results"][0]
    assert res0["file_name_edit"] == str(FROG_PY)
    assert res0["file_text"] == text_expected + "\n", res0["file_text"]
    print(colored("test01_rewrite_whole_file PASSED", "green"))


def test01_new_file():
    text_expected = "# FROG"
    FN = str(FROG_PY) + ".temp"
    ticket_text = \
f"""📍NEW_FILE 001 {FN}
```python
{text_expected}
```
"""
    messages = make_messages(ticket_text)
    resp = patch_request(messages, ["001"])

    res0 = resp["results"][0]
    assert res0["file_name_add"] == str(FN)
    assert res0["file_text"] == text_expected + "\n", res0["file_text"]
    print(colored("test01_new_file PASSED", "green"))


def test01_partial_edit():
    text_expected = (TEST11_DATA / "toad_partial_edit_01.py").read_text()
    ticket_text = \
f"""📍PARTIAL_EDIT 001 {TOAD_ORIG}
```python
DT = 0.1
```
"""
    messages = make_messages(ticket_text)
    resp = patch_request(messages, ["001"])

    res0 = resp["results"][0]
    assert res0["file_name_edit"] == str(TOAD_ORIG)
    assert res0["file_text"] == text_expected
    print(colored("test01_partial_edit PASSED", "green"))


def test02_partial_edit():
    text_expected = (TEST11_DATA / "toad_partial_edit_02.py").read_text()
    ticket_text = \
f"""📍PARTIAL_EDIT 001 {TOAD_ORIG}
```python
    def croak(self, x, y, n_times):
        for _ in range(n_times):
            print("croak")
            echo_times = self.calculate_echo_time(x, y)
            for t in echo_times:
                print(f"Echo after {{t:.2f}} seconds")
```
"""
    messages = make_messages(ticket_text)
    resp = patch_request(messages, ["001"])

    res0 = resp["results"][0]
    assert res0["file_name_edit"] == str(TOAD_ORIG), res0
    assert res0["file_text"] == text_expected, print(res0["file_text"])
    print(colored("test02_partial_edit PASSED", "green"))


def test01_add_to_file():
    ticket_text = \
f"""📍ADD_TO_FILE 001 {TOAD_ORIG} BEFORE Toad
```python
def hello_toad():
    print("Hello Toad!")
```
"""
    messages = make_messages(ticket_text)
    resp = patch_request(messages, ["001"])


def test02_add_to_file():
    ticket_text = \
        f"""📍ADD_TO_FILE 001 {TOAD_ORIG} AFTER Toad
```python
def hello_toad():
    print("Hello Toad!")
```
"""
    messages = make_messages(ticket_text)
    resp = patch_request(messages, ["001"])


def test03_add_to_file():
    ticket_text = \
f"""📍ADD_TO_FILE 001 {TOAD_ORIG} BEFORE Toad::bounce_off_banks
"""


def test04_add_to_file():
    ticket_text = \
f"""📍ADD_TO_FILE 001 {TOAD_ORIG} AFTER Toad::bounce_off_banks
"""


def test01_rewrite_function():
    ticket_text = \
f"""📍REWRITE_FUNCTION 001 {TOAD_ORIG} FUNC standalone_jumping_function
"""


if __name__ == "__main__":
    # test01_rewrite_whole_file()
    # test01_new_file()
    # test01_partial_edit()
    test02_partial_edit()
