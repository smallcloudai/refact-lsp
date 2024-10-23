import requests
import termcolor
import json


BASE_URL = "http://127.0.0.1:8001"

def test_set_and_get_silly_message():
    cthread_id = "silly_thread_123"
    message_index = 42
    silly_message = {
        "role": "user",
        "content": "Why did the scarecrow win an award? Because he was outstanding in his field!",
    }

    set_response = requests.post(
        f"{BASE_URL}/v1/choredb-chat-message-set",
        json={
            "cthread_id": cthread_id,
            "i": message_index,
            "message": silly_message
        },
        headers={"Content-Type": "application/json"}
    )

    assert set_response.status_code == 200, f"Failed to set message. Status code: {set_response.status_code}"
    print(termcolor.colored("Message set successfully", "green"))

    get_response = requests.get(
        f"{BASE_URL}/v1/choredb-chat-message-get?cthread_id={cthread_id}&i={message_index}"
    )

    assert get_response.status_code == 200, f"Failed to get message. Status code: {get_response.status_code}"
    retrieved_message = get_response.json()

    assert retrieved_message == silly_message, "Retrieved message doesn't match the one we set"
    print(termcolor.colored("Retrieved message matches the one we set", "green"))

    print(termcolor.colored(f"\nHere's a silly joke for you:", "cyan"))
    print(termcolor.colored(f"{retrieved_message['content']}", "yellow"))

    print(termcolor.colored("\nTEST PASSED", "green", attrs=["bold"]))

if __name__ == "__main__":
    test_set_and_get_silly_message()
