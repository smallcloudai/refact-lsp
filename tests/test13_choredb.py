import aiohttp
import asyncio
import termcolor
import json

BASE_URL = "http://127.0.0.1:8001"

silly_message = {
    "role": "user",
    "content": "Why did the scarecrow win an award? Because he was outstanding in his field!",
}


async def listen_to_sse(session):
    async with session.post(
        f"{BASE_URL}/db_v1/cthread-sub",
        json={"limit": 5, "quicksearch": ""},
        headers={"Content-Type": "application/json"},
    ) as response:
        if response.status != 200:
            print(termcolor.colored(f"Failed to connect to SSE. Status code: {response.status}", "red"))
            return
        print(termcolor.colored("Connected to SSE", "green"))
        async for line in response.content:
            if line:
                decoded_line = line.decode('utf-8')
                print(termcolor.colored(f"Received SSE: {decoded_line}", "yellow"))


async def update_cthread(session, cthread_id):
    upd_response = await session.post(
        f"{BASE_URL}/db_v1/cthread-update",
        json={
            "cthread_id": cthread_id,
            "cthread_title": "Hello world!!!"
        },
        headers={"Content-Type": "application/json"}
    )

    assert upd_response.status == 200, f"Failed to set cthread title. Status code: {upd_response.status}"
    print(termcolor.colored("cthread_update", "green"))


async def main():
    cthread_id = "silly_thread_123"
    headers = {"Content-Type": "application/json"}

    async with aiohttp.ClientSession() as session:
        sse_task = asyncio.create_task(listen_to_sse(session))
        update_task = asyncio.create_task(update_cthread(session, cthread_id))
        await asyncio.gather(sse_task, update_task)

    print(termcolor.colored("\nTEST PASSED", "green", attrs=["bold"]))


if __name__ == "__main__":
    asyncio.run(main())