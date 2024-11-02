import aiohttp
import asyncio
import termcolor
import json
import argparse
import time

BASE_URL = "http://127.0.0.1:8001"

silly_message = {
    "role": "user",
    "content": "Why did the scarecrow win an award? Because he was outstanding in his field!",
}


async def listen_to_sse(session):
    async with session.post(
        f"{BASE_URL}/db_v1/cthreads-sub",
        json={"limit": 5, "quicksearch": ""},
        headers={"Content-Type": "application/json"},
    ) as response:
        if response.status != 200:
            print(termcolor.colored(f"Failed to connect to SSE. Status code: {response.status}", "red"))
            return
        print(termcolor.colored("Connected to SSE", "green"))
        async for line in response.content:
            if line:
                decoded_line = line.decode('utf-8').strip()
                if decoded_line:
                    print(termcolor.colored(decoded_line, "yellow"))
                    print()


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


async def main(only_sub=False, only_update=False):
    cthread_id = f"silly_thread_{int(time.time())}"  # Modified this line
    headers = {"Content-Type": "application/json"}

    async with aiohttp.ClientSession() as session:
        tasks = []

        if only_sub or (not only_sub and not only_update):
            tasks.append(asyncio.create_task(listen_to_sse(session)))

        if only_update or (not only_sub and not only_update):
            tasks.append(asyncio.create_task(update_cthread(session, cthread_id)))

        await asyncio.gather(*tasks)

    print(termcolor.colored("\nTEST PASSED", "green", attrs=["bold"]))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description="ChoreDB test script")
    parser.add_argument("--only-sub", action="store_true", help="Run only the subscription part")
    parser.add_argument("--only-update", action="store_true", help="Run only the update part")
    args = parser.parse_args()

    asyncio.run(main(only_sub=args.only_sub, only_update=args.only_update))
