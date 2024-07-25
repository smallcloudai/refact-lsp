import re
import json
import subprocess

from refact import chat_client
from refact.chat_client import print_block
from refact.chat_client import print_exception
from refact.chat_client import print_messages
from swe.steps import Step

from pathlib import Path
from typing import List, Set


SYSTEM_MESSAGE = """
You're Refact Dev a prefect AI assistant.

You plan is to:
- Look through the user's problem statement and files structure.
- If needed collect context using definition and references tools.
- Call patch tool to produce a patch that solves given problem.

Rules of patch tool using:
- Choose exact one filename to patch.
- You should solve the problem with exact one patch tool call.

How patch tool's todo argument must looks like:
- Todo should contain the plan how to solve given problem with detailed description of each step.
- Add all needed symbols, their definitions and other code that should help with problem solving.
"""


class ProducePatchStep(Step):

    def __init__(self, choices: int, *args, **kwargs):
        super().__init__(*args, **kwargs)
        self._choices = choices

    @property
    def _tools(self) -> Set[str]:
        return {
            "patch",
        }

    @staticmethod
    def _extract_filenames(text: str, filter_tests: bool = False) -> Set[str]:
        pattern = r'\b(?:[a-zA-Z]:\\|/)?(?:[\w-]+[/\\])*[\w-]+\.\w+\b'
        filenames = set(re.findall(pattern, text))
        if filter_tests:
            filenames = {f for f in filenames if "test" not in f.lower()}
        return filenames

    async def _patch_generate(self, message: chat_client.Message, repo_name: Path):
        if message.role != "diff":
            raise RuntimeError("not a diff message")
        formatted_diff = json.loads(message.content)
        await chat_client.diff_apply(self._base_url, chunks=formatted_diff, apply=[True] * len(formatted_diff))
        result = subprocess.check_output(["git", "--no-pager", "diff"], cwd=str(repo_name))
        await chat_client.diff_apply(self._base_url, chunks=formatted_diff, apply=[False] * len(formatted_diff))
        return result.decode()

    async def _attempt(self, message: chat_client.Message, repo_name: Path) -> str:
        patch_tool_messages = await self._query([message], only_deterministic_messages=True)
        self._trajectory.extend(print_messages(patch_tool_messages))
        for message in patch_tool_messages:
            if message.role == "diff":
                return await self._patch_generate(message, repo_name)
        raise RuntimeError(f"expected a diff message")

    async def process(self, problem_statement: str, related_files: List[str], repo_path: Path, **kwargs) -> List[str]:
        paths = ",".join([str(repo_path / filename) for filename in related_files])
        files_tool_call_dict = chat_client.ToolCallDict(
            id=chat_client.gen_function_call_id(),
            function=chat_client.FunctionDict(arguments='{"paths":"' + paths + '"}', name='files_skeleton'),
            type='function')
        messages = [
            chat_client.Message(role="system", content=SYSTEM_MESSAGE),
            chat_client.Message(role="user", content=f"Problem statement:\n\n{problem_statement}"),
            chat_client.Message(role="assistant", finish_reason="tool_calls", tool_calls=[files_tool_call_dict]),
        ]
        tree_tool_messages = await self._query(messages, only_deterministic_messages=True)
        assert len(tree_tool_messages) == 1 and tree_tool_messages[0].role == "tool"
        messages.extend(tree_tool_messages)
        self._trajectory.extend(print_messages(messages))

        results = set()
        for idx, new_messages in enumerate(await self._query_choices(messages, self._choices)):
            self._trajectory.append(print_block("choice", idx + 1))
            self._trajectory.extend(print_messages(new_messages))
            try:
                results.add(await self._attempt(new_messages[-1], repo_path.absolute()))
            except Exception as e:
                self._trajectory.append(print_exception(e, trace=True))
        return list(results)
