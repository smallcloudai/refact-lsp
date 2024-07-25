import random

from swe.steps import Step
from refact import chat_client
from refact.chat_client import print_block
from refact.chat_client import print_messages

from pathlib import Path
from collections import Counter
from typing import List, Set


SYSTEM_MESSAGE = f"""
You are Refact Dev, an auto coding assistant.

You plan is to:
- Look through the user's problem statement, given code context and the solutions.
- Speculate about given solutions and choose one that perfectly solves the problem.

Your answer should contain speculation and result solution name (e.g. Solution 55, Solution 9, etc.)
Result should be at the end of the answer.
For example:

Speculation about solution
...
Result
```
Solution 99
```
"""


class ChooseSolutionStep(Step):
    def __init__(self, choices: int, *args, **kwargs):
        super().__init__(*args, **kwargs)
        self._choices = choices

    @property
    def _tools(self) -> Set[str]:
        return set()

    @staticmethod
    def _extract_solution(message: chat_client.Message, n_solutions: int) -> int:
        if message.role != "assistant":
            raise RuntimeError(f"unexpected message role '{message.role}' for answer")
        if not isinstance(message.content, str):
            raise RuntimeError(f"unexpected content type '{type(message.content)}' for answer")
        result_text = message.content.lower().split("result")[-1]
        for line in result_text.split("\n"):
            line = line.strip()
            if not line.startswith("solution"):
                continue
            solution_idx = int(line.split("solution")[1].strip()) - 1
            if 0 <= solution_idx < n_solutions:
                return solution_idx
        raise RuntimeError(f"can't extract solution from '{message.content}'")

    async def process(
            self,
            problem_statement: str,
            related_files: List[str],
            model_patches: List[str],
            repo_path: Path,
            **kwargs) -> str:
        if not model_patches:
            raise RuntimeError("no patches for problem")
        if len(model_patches) < 2:
            return model_patches[0]

        user_message_parts = [
            "Problem statement:",
            problem_statement,
        ]
        random.shuffle(model_patches)
        for idx, model_patch in enumerate(model_patches, start=1):
            user_message_parts.extend([
                f"Solution {idx}:",
                model_patch,
            ])

        # TODO: I'm not sure we need to add files that are not in given patches
        paths = ",".join([str(repo_path / filename) for filename in related_files])
        files_tool_call_dict = chat_client.ToolCallDict(
            id=chat_client.gen_function_call_id(),
            function=chat_client.FunctionDict(arguments='{"paths":"' + paths + '"}', name='files_skeleton'),
            type='function')
        messages = [
            chat_client.Message(role="system", content=SYSTEM_MESSAGE),
            chat_client.Message(role="user", content="\n\n".join(user_message_parts)),
            chat_client.Message(role="assistant", finish_reason="tool_calls", tool_calls=[files_tool_call_dict]),
        ]
        tool_messages = await self._query(messages, only_deterministic_messages=True)
        assert len(tool_messages) == 1 and tool_messages[0].role == "tool"
        messages.extend(tool_messages)
        self._trajectory.extend(print_messages(messages))

        counter = Counter()
        for idx, new_messages in enumerate(await self._query_choices(messages, self._choices)):
            self._trajectory.append(print_block("choice", idx + 1))
            self._trajectory.extend(print_messages(new_messages))
            try:
                counter.update([self._extract_solution(new_messages[-1], len(model_patches))])
            except:
                continue

        if not counter:
            raise RuntimeError("can't choose a solution")

        return model_patches[counter.most_common()[0][0]]
