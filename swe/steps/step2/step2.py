import re
import json
import subprocess

from refact import chat_client
from refact.chat_client import print_block
from refact.chat_client import print_exception
from refact.chat_client import print_messages
from swe.steps import Step

from collections import Counter
from pathlib import Path
from typing import List, Set


CONTEXT_SYSTEM_MESSAGE = f"""
You're Refact Dev a prefect AI assistant.

You should collect all needed context to solve the problem.
- Look through the user's problem statement and given files structure.
- Collect additional context using definition and references tools if needed.
- Call tools in parallel as much as it possible.
"""

PATCH_SYSTEM_MESSAGE = f"""
You're Refact Dev a prefect AI assistant.

You should solve the problem using given context and patch tool:
- Choose relevant file to patch.
- Introduce a plan that will solve the problem.
- Simultaneously call the patch tool to produce a patch.

Rules of patch tool using:
- Choose exact one filename to patch.
- You should solve the problem with exact one patch tool call per message.
- Patch command doesn't have your context so you need to pass all relevant symbols and write accurate todo.
- Todo should contain the plan how to solve given problem with detailed description of each step and warnings about possible problems with solution.
"""


PATCH_TODO_REMINDER = f"""
A reminder of patch generation:
- Make sure you added all imports if it needed.
- Do not add anything that is not related to the problem.
- Your patch should be minimalistic, never try to add unnecessary code.
- If it possible use native language objects.

If you see that you can't solve the problem in given file with provided context just refuse patch generation!
"""


class ProducePatchStep(Step):

    def __init__(self, context_choices: int, patch_choices: int, *args, **kwargs):
        super().__init__(*args, **kwargs)
        self._context_choices = context_choices
        self._patch_choices = patch_choices
        self._active_tools = set()

    @property
    def _tools(self) -> Set[str]:
        return self._active_tools

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

    async def _patch(self, message: chat_client.Message, repo_name: Path, problem_statement: str) -> str:
        function_dict = message.tool_calls[0].function
        if function_dict.name != "patch":
            raise RuntimeError("not a patch tool call")
        args = json.loads(function_dict.arguments)
        if len(args.get("paths", "").split(",")) != 1:
            raise RuntimeError("patch tool call should edit exactly one filename")
        if not args.get("todo", ""):
            raise RuntimeError("patch tool should contain todo")
        args["todo"] = "\n\n".join([
            "Original problem:", problem_statement,
            "Plan:", args["todo"],
            PATCH_TODO_REMINDER,
        ])
        function_dict.arguments = json.dumps(args)
        self._trajectory.extend(print_messages([message]))
        patch_tool_messages = await self._query([message], only_deterministic_messages=True)
        self._trajectory.extend(print_messages(patch_tool_messages))
        for message in patch_tool_messages:
            if message.role == "diff":
                return await self._patch_generate(message, repo_name)
        raise RuntimeError(f"expected a diff message")

    async def _deterministic_tool_call_messages(
            self, functions: List[chat_client.FunctionDict]) -> List[chat_client.Message]:
        tool_calls = [
            chat_client.ToolCallDict(id=chat_client.gen_function_call_id(), function=function, type='function')
            for function in functions
        ]
        messages = [
            chat_client.Message(role="assistant", finish_reason="tool_calls", tool_calls=tool_calls),
        ]
        tool_messages = await self._query(messages, only_deterministic_messages=True)
        return messages + tool_messages

    async def _collect_context(
            self,
            problem_statement: str,
            related_files: List[str],
            repo_path: Path) -> List[chat_client.Message]:
        self._active_tools = {
            "definition",
            "references",
        }
        messages = [
            chat_client.Message(role="system", content=CONTEXT_SYSTEM_MESSAGE),
            chat_client.Message(role="user", content=f"Problem statement:\n\n{problem_statement}"),
        ]

        paths = ",".join([str(repo_path / filename) for filename in related_files])
        messages.extend(
            await self._deterministic_tool_call_messages([
                chat_client.FunctionDict(arguments='{"paths":"' + paths + '"}', name='files_skeleton')
            ])
        )
        self._trajectory.extend(print_messages(messages))

        function_dict_counter = Counter()
        for idx, new_messages in enumerate(await self._query_choices(messages, self._context_choices)):
            self._trajectory.append(print_block("context choice", idx + 1))
            self._trajectory.extend(print_messages(new_messages))
            try:
                def _normalize(args: str):
                    try:
                        return json.dumps(json.loads(args))
                    except:
                        return args

                function_dict_counter.update([
                    chat_client.FunctionDict(
                        name=tool_call_dict.function.name,
                        arguments=_normalize(tool_call_dict.function.arguments),
                    )
                    for tool_call_dict in new_messages[-1].tool_calls
                    if tool_call_dict.type == "function"
                ])
            except Exception as e:
                self._trajectory.append(print_exception(e, trace=True))

        if function_dict_counter:
            tool_call_messages = await self._deterministic_tool_call_messages([
                function_dict for function_dict, _ in function_dict_counter.most_common()
            ])
            self._trajectory.extend(print_messages(tool_call_messages))
            messages += tool_call_messages

        return messages[1:]

    async def _collect_patches(
            self,
            problem_statement: str,
            repo_path: Path,
            context_messages: List[chat_client.Message]):
        self._active_tools = {
            "patch",
        }
        messages = [
            chat_client.Message(role="system", content=PATCH_SYSTEM_MESSAGE),
            *context_messages,
        ]
        results = list()
        for idx, new_messages in enumerate(await self._query_choices(messages, self._patch_choices)):
            self._trajectory.append(print_block("patch choice", idx + 1))
            self._trajectory.extend(print_messages(new_messages))
            try:
                results.append(await self._patch(new_messages[-1], repo_path.absolute(), problem_statement))
            except Exception as e:
                self._trajectory.append(print_exception(e, trace=True))
        return results

    async def process(self, problem_statement: str, related_files: List[str], repo_path: Path, **kwargs) -> List[str]:
        context_messages = await self._collect_context(problem_statement, related_files, repo_path)
        results = await self._collect_patches(problem_statement, repo_path, context_messages)
        return results
