import re
import json
import subprocess
import asyncio

from refact.chat_client import Message
from refact.chat_client import FunctionDict
from refact.chat_client import diff_apply
from refact.chat_client import print_block
from refact.chat_client import print_exception
from refact.chat_client import print_messages
from swe.steps import Step

from collections import Counter
from pathlib import Path
from typing import List, Set, Dict, Tuple, Any


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

    def __init__(self, patch_choices: int, *args, **kwargs):
        super().__init__(*args, **kwargs)
        self._patch_choices = patch_choices

    @property
    def _tools(self) -> Set[str]:
        return {
            "patch",
        }

    @staticmethod
    async def _lint(filename: Path) -> bool:
        process = await asyncio.create_subprocess_exec(
            "flake8",
            "--select=E9,F821,F823,F831,F406,F407,F701,F702,F704,F706",
            "--show-source",
            "--isolated", str(filename),
        )
        await process.communicate()
        return process.returncode == 0

    @staticmethod
    def _extract_filenames(text: str, filter_tests: bool = False) -> Set[str]:
        pattern = r'\b(?:[a-zA-Z]:\\|/)?(?:[\w-]+[/\\])*[\w-]+\.\w+\b'
        filenames = set(re.findall(pattern, text))
        if filter_tests:
            filenames = {f for f in filenames if "test" not in f.lower()}
        return filenames

    async def _patch_generate(self, message: Message, repo_name: Path) -> Tuple[str, int, Dict[str, Any]]:
        if message.role != "diff":
            raise RuntimeError("not a diff message")
        formatted_diff = json.loads(message.content)
        await diff_apply(self._base_url, chunks=formatted_diff, apply=[True] * len(formatted_diff))
        result = subprocess.check_output(["git", "--no-pager", "diff"], cwd=str(repo_name))
        is_linted = all([
            await self._lint(filename)
            for filename in set([d["file_name"] for d in formatted_diff])
        ])
        await diff_apply(self._base_url, chunks=formatted_diff, apply=[False] * len(formatted_diff))
        # TODO: we need to add all patches from patch tool as messages with count attr
        return result.decode(), 1, {"is_linted": is_linted, "formatted_diff": formatted_diff}

    async def _patch(self, message: Message, repo_name: Path, problem_statement: str) -> Tuple[Counter, Dict]:
        function_dict = message.tool_calls[0].function
        if function_dict.name != "patch":
            raise RuntimeError("not a patch tool call")
        args = json.loads(function_dict.arguments)
        if len(args.get("paths", "").split(",")) != 1:
            raise RuntimeError("patch tool call should edit exactly one filename")
        if not args.get("todo", ""):
            raise RuntimeError("patch tool should contain todo")
        todo = args["todo"]
        args["todo"] = "\n\n".join([
            "Original problem:", problem_statement,
            "Plan:", todo,
            PATCH_TODO_REMINDER,
        ])
        function_dict.arguments = json.dumps(args)
        message.tool_calls = message.tool_calls[:1]
        self._trajectory.extend(print_messages([message]))
        patch_tool_messages = await self._query([message], only_deterministic_messages=True)
        self._trajectory.extend(print_messages(patch_tool_messages))

        results = []
        for message in patch_tool_messages:
            if message.role != "diff":
                continue
            model_patch, count, raw_diff_info = await self._patch_generate(message, repo_name)
            results.append((model_patch, count, raw_diff_info))

        model_patches = Counter({
            model_patch: count
            for model_patch, count, result_info in results
            if result_info["is_linted"] and model_patch
        })
        if not model_patches:
            raise RuntimeError(f"expected a diff message")
        return model_patches, {"todo": todo, "results": results}

    async def _collect_patches(
            self,
            problem_statement: str,
            context_messages: List[Message],
            repo_path: Path,
    ):
        messages = [
            Message(role="system", content=PATCH_SYSTEM_MESSAGE),
            Message(role="user", content=f"Problem statement:\n\n{problem_statement}"),
            *context_messages,
        ]

        patch_count = 0
        attempt_results = []
        model_patches_counter = Counter()
        for idx, new_messages in enumerate(await self._query_choices(messages, self._patch_choices)):
            self._trajectory.append(print_block("patch choice", idx + 1))
            self._trajectory.extend(print_messages(new_messages))
            try:
                model_patches, results = await self._patch(new_messages[-1], repo_path.absolute(), problem_statement)
                model_patches_counter.update(model_patches)
                attempt_results.append(results)
                patch_count += 1
                if patch_count == 3:
                    break
            except Exception as e:
                self._trajectory.append(print_exception(e, trace=True))

        # NOTE: we need to improve patches of counter
        # 1. count score over each attempt (gives higher probability)
        # 2. patch normalization before linting
        model_patches_with_scores = [
            (p, cnt / sum(model_patches_counter.values()))
            for p, cnt in model_patches_counter.most_common()
        ]
        return model_patches_with_scores, attempt_results

    async def process(
            self,
            problem_statement: str,
            context_files: List[str],
            context_symbols: List[str],
            to_change_files: List[str],
            repo_path: Path,
            **kwargs) -> Tuple[List, List]:
        context_messages = await self._deterministic_tool_call_messages([
            FunctionDict(name="cat", arguments=json.dumps({
                "paths": ",".join([str(repo_path / filename) for filename in context_files]),
                "symbols": ",".join(context_symbols),
                "skeleton": True,
            }))
        ])
        if to_change_files:
            notes_message = "\n".join([
                "Most likely you should patch:",
                *to_change_files,
            ])
            context_messages.append(Message(role="user", content=notes_message))
        return await self._collect_patches(
            problem_statement=problem_statement,
            context_messages=context_messages,
            repo_path=repo_path)
