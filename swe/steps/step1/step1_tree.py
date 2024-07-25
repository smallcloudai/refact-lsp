import re

from refact import chat_client
from refact.chat_client import print_messages
from refact.chat_client import print_block
from swe.steps import Step

from collections import Counter
from pathlib import Path
from typing import Set, List


SYSTEM_MESSAGE = """
You're Refact Dev a prefect AI assistant.

You plan is to:
- Look through the user's problem statement.
- Call tree tool to obtain repository structure.
- Provide a list of files that one would need to edit to fix the problem.

Please only provide the full path and return at least 5 files.
The returned files should be separated by new lines ordered by most to least important and wrapped with ```
For example:
```
file1.py
file2.py
```
"""


class ExploreRepoStep(Step):

    def __init__(self, choices: int, *args, **kwargs):
        super().__init__(*args, **kwargs)
        self._choices = choices

    @staticmethod
    def _extract_filenames(text: str, repo_root, filter_tests: bool = False) -> List[str]:
        pattern = r'\b(?:[a-zA-Z]:\\|/)?(?:[\w-]+[/\\])*[\w-]+\.\w+\b'
        filenames = set([
            filename.replace(repo_root.lstrip("/"), "").lstrip("/")
            for filename in re.findall(pattern, text)
        ])
        if filter_tests:
            filenames = {f for f in filenames if "test" not in f.lower()}
        return list(filenames)

    @property
    def _tools(self) -> Set[str]:
        return set()

    def _attempt(self, message: chat_client.Message, repo_path: Path) -> List[str]:
        if message.role != "assistant":
            raise RuntimeError(f"unexpected message role '{message.role}' for answer")
        if not isinstance(message.content, str):
            raise RuntimeError(f"unexpected content type '{type(message.content)}' for answer")
        found_files = self._extract_filenames(message.content, str(repo_path))
        if len(found_files) == 0:
            raise RuntimeError(f"no files found")
        return found_files

    async def process(self, problem_statement: str, repo_path: Path, **kwargs) -> List[str]:
        tree_tool_call_dict = chat_client.ToolCallDict(
            id=chat_client.gen_function_call_id(),
            function=chat_client.FunctionDict(arguments='{}', name='tree'),
            type='function')
        messages = [
            chat_client.Message(role="system", content=SYSTEM_MESSAGE),
            chat_client.Message(role="user", content=f"Problem statement:\n\n{problem_statement}"),
            chat_client.Message(role="assistant", finish_reason="tool_calls", tool_calls=[tree_tool_call_dict]),
        ]
        tree_tool_messages = await self._query(messages, only_deterministic_messages=True)
        assert len(tree_tool_messages) == 1 and tree_tool_messages[0].role == "tool"
        messages.extend(tree_tool_messages)
        self._trajectory.extend(print_messages(messages))

        file_counter = Counter()
        for idx, new_messages in enumerate(await self._query_choices(messages, self._choices)):
            self._trajectory.append(print_block("choice", idx + 1))
            self._trajectory.extend(print_messages(new_messages))
            try:
                file_counter.update(self._attempt(new_messages[-1], repo_path))
            except:
                continue
        found_files = [k for k, _ in file_counter.most_common(5)]
        if not found_files:
            raise RuntimeError(f"no files found")
        return found_files
