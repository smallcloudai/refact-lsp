import json

from refact.chat_client import FunctionDict
from refact.chat_client import print_messages
from swe.steps import Step

from pathlib import Path
from typing import List, Dict, Any, Set


class Locate(Step):
    def __init__(self, *args, **kwargs):
        super().__init__(*args, **kwargs)

    @property
    def _tools(self) -> Set[str]:
        return set()

    async def process(self, problem_statement: str, repo_path: Path, **kwargs) -> Dict[str, Any]:
        messages = await self._deterministic_tool_call_messages([
            FunctionDict(name="locate", arguments=json.dumps({
                "problem_statement": problem_statement,
                # "problem_statement": f"Problem statement:\n\n{problem_statement}",
            }))
        ])
        self._trajectory.extend(print_messages(messages))

        try:
            results: Dict[str, List[Any]] = json.loads(messages[-1].content)
        except Exception as e:
            raise RuntimeError(f"content is not decodable as json:\n{messages[-1].content}\nError: {e}")

        files_list = results.get("files", {})
        symbols_list = results.get("symbols", [])

        if not files_list:
            raise RuntimeError(f"no files found")

        if not isinstance(files_list[0], dict):
            raise RuntimeError(f"files list is not a list of dicts")

        context_files, to_change_files = [], []
        for info in files_list:
            file_path = info["file_path"]
            # TODO: for now tool returns absolute paths
            if file_path.startswith(str(repo_path)):
                file_path = str(Path(file_path).relative_to(repo_path))
            # TODO: the tool can return not existing files
            if not (repo_path / file_path).exists():
                continue
            context_files.append(file_path)
            if info["reason"] == "to_change":
                to_change_files.append(file_path)

        # TODO: return description of the files
        return {
            "context_files": context_files,
            "context_symbols": symbols_list,
            "to_change_files": to_change_files,
        }


