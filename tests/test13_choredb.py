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


async def listen_to_cthreads(session):
    async with session.post(
        f"{BASE_URL}/db_v1/cthreads-sub",
        json={"limit": 100, "quicksearch": ""},
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

async def listen_to_cmessages(session, thread_id):
    async with session.post(
        f"{BASE_URL}/db_v1/cmessages-sub",
        json={"cmessage_belongs_to_cthread_id": thread_id},
        headers={"Content-Type": "application/json"},
    ) as response:
        if response.status != 200:
            print(termcolor.colored(f"Failed to connect to SSE. Status code: {response.status}", "red"))
            return
        print(termcolor.colored("Connected to SSE %s" % thread_id, "green"))
        async for line in response.content:
            if line:
                decoded_line = line.decode('utf-8').strip()
                if decoded_line:
                    print(termcolor.colored(decoded_line, "yellow"))
                    print()


async def various_updates_generator(session, cthread_id):
    r = await session.post(f"{BASE_URL}/db_v1/cthread-update",
        json={
            "cthread_id": cthread_id,
            "cthread_title": "Hello world!!!",
            "cthread_anything_new": True,
        },
    )
    assert r.status == 200, f"oops:\n{r}"

    r = await session.post(f"{BASE_URL}/db_v1/cmessage-update",
        json={
            "cmessage_belongs_to_cthread_id": cthread_id,
            "cmessage_alt": 0,
            "cmessage_num": 0,
            "cmessage_prev_alt": -1,
            "cmessage_usage_model": "gpt-3.5",
            "cmessage_json": "{ \"something\": \"fishy\" }"
        },
    )
    assert r.status == 200, f"oops:\n{r}"

    print(termcolor.colored("updates over", "green"))


async def main(only_sub=False, only_update=False):
    cthread_id = f"silly_thread_{int(time.time())}"
    headers = {"Content-Type": "application/json"}

    async with aiohttp.ClientSession() as session:
        tasks = []
        if only_sub or (not only_sub and not only_update):
            tasks.append(asyncio.create_task(listen_to_cthreads(session)))
            tasks.append(asyncio.create_task(listen_to_cmessages(session, cthread_id)))
        if only_update or (not only_sub and not only_update):
            tasks.append(asyncio.create_task(various_updates_generator(session, cthread_id)))
        await asyncio.gather(*tasks)

    print(termcolor.colored("\nTEST OVER", "green", attrs=["bold"]))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description="ChoreDB test script")
    parser.add_argument("--only-sub", action="store_true", help="Run only the subscription part")
    parser.add_argument("--only-update", action="store_true", help="Run only the update part")
    args = parser.parse_args()

    asyncio.run(main(only_sub=args.only_sub, only_update=args.only_update))
