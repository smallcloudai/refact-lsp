import asyncio
import aiohttp
import json, time
from typing import Dict, Any, Optional, Tuple

base_url = "http://127.0.0.1:8001"


async def mem_add(session: aiohttp.ClientSession, mem_type: str, goal: str, project: str, payload: str) -> Dict[str, Any]:
    url = f"{base_url}/v1/mem-add"
    data = {
        "mem_type": mem_type,
        "goal": goal,
        "project": project,
        "payload": payload
    }
    async with session.post(url, json=data) as response:
        return await response.json()


async def mem_block_until_vectorized(session: aiohttp.ClientSession) -> Tuple[Dict[str, Any], float]:
    url = f"{base_url}/v1/mem-block-until-vectorized"
    t0 = time.time()
    async with session.get(url) as response:
        return (await response.json(), time.time() - t0)


async def mem_update_used(session: aiohttp.ClientSession, memid: str, correct: float, useful: float) -> Dict[str, Any]:
    url = f"{base_url}/v1/mem-update-used"
    data = {
        "memid": memid,
        "correct": correct,
        "useful": useful
    }
    async with session.post(url, json=data) as response:
        return await response.json()


async def mem_erase(session: aiohttp.ClientSession, memid: str) -> Dict[str, Any]:
    url = f"{base_url}/v1/mem-erase"
    data = {
        "memid": memid
    }
    async with session.post(url, json=data) as response:
        return await response.json()


async def mem_query(session: aiohttp.ClientSession, goal: str, project: str, top_n: Optional[int] = 5) -> Tuple[int, Dict[str, Any]]:
    url = f"{base_url}/v1/mem-query"
    data = {
        "goal": goal,
        "project": project,
        "top_n": top_n
    }
    async with session.post(url, json=data) as response:
        return response.status, await response.json()


async def test_memory_operations():
    async with aiohttp.ClientSession() as session:
        m0 = await mem_add(session, "seq-of-acts", "compile", "proj1", "Wow, running cargo build on proj1 was successful!")
        m1 = await mem_add(session, "proj-fact", "compile", "proj1", "Looks like proj1 is written in fact in Rust.")
        m2 = await mem_add(session, "seq-of-acts", "compile", "proj2", "Wow, running cargo build on proj2 was successful!")
        m3 = await mem_add(session, "proj-fact", "compile", "proj2", "Looks like proj2 is written in fact in Rust.")
        print("Added memories:\n%s\n%s\n%s\n%s" % (m0, m1, m2, m3))

        bl, bl_t = await mem_block_until_vectorized(session)
        print("Block result: %0.1fs %s" % (bl_t, bl))

        update_result = await mem_update_used(session, m1["memid"], 0.95, 0.85)
        print("Updated memory:", update_result)

        erase_result = await mem_erase(session, m0["memid"])
        print("Erased memory:", erase_result)

        await asyncio.sleep(3)
        http_status, query_result = await mem_query(session, "compile", "proj1")
        print("Query result: %s\n%s" % (http_status, json.dumps(query_result, indent=4)))

        # You can add more assertions here to verify the results
        # For example:
        # assert "memid" in m0, "Memory addition failed"
        # assert update_result["status"] == "success", "Memory update failed"
        # assert erase_result["status"] == "success", "Memory erasure failed"


if __name__ == "__main__":
    asyncio.run(test_memory_operations())
