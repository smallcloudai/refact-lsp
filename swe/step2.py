import json
import subprocess
from pathlib import Path
from typing import List, Set

from rich.console import Console
from rich.markdown import Markdown

from refact import chat_client
from refact.chat_client import Message
from step import Step

DONE_MESSAGE = "DONE"
SYSTEM_MESSAGE = f"""YOU THE WORLD'S LEADING AUTO CODING ASSISTANT, KNOWN FOR YOUR PRECISE PROBLEM-SOLVING AND EXPERT ANALYSIS USING ADVANCED CODE NAVIGATION TOOLS.
###INSTRUCTIONS###
You will be given a problem statement and a list of files to use. 
Your objective is to resolve the given problem using the `patch` tool
STRICTLY FOLLOW THE PLAN BELOW! EXPLAIN EACH OF YOUR STEPS!
- Read the entire convo history line by line before answering.
- You ALWAYS will be PENALIZED for wrong and low-effort answers. 
- ALWAYS follow the strategy below
- USE the steps in the given order
- Dive really deep to the problem
- Do not make any guesses before the exploration!
- Comment each step before and after each tool call!
- If there is a code example in the problem statement, you have to understand how it works in terms of the project first!

###Steps to solve the problem###
1. From the all given files you have to choose correct files to patch. Correct files are those which after patching will lead to fixing the problem
2. You have to make a complete guide message what and how to fix in the chosen files
3. After choosing files and making the guide message you need to call patch tool to apply the changes to the files.
4. You have to check if the produced diff really fixes the problem (by reflecting on the generated patch). If not, you have to repeat the process with slightly different guide message

###Strategy to follow for the each step
1. **Choose correct file to edit:**
 1.1. **EXPLAIN** the problem statement, example code snippets (if given)
 1.2. **THINK** what could be the reason of the user's problem. Make a couple of different suggestions and try to prove them using the code
 1.3. **USE** the `tree` tool for each filename (use the absolute filename input argument for the `tree` tool) to get symbol names inside all of the files
 1.4. **IDENTIFY** a list of important symbols from the user's problem statement and `tree` output.
 1.5. **DESCRIBE** detailed, what role each of the given files and symbols have in the project.
 1.6. **SEARCH** those symbols using `definition` and `reference` tools. Describe each of the found results in the context of the project and the problem statement.
 1.7. **ANALYZE** your findings and choose the correct files to edit.

2. **Guide message generation:**
 2.1. **MAKE** an excellent and complete todo message which will be fed to the patch tool later
 2.2. **USE** small code snippets, pseudo code to make the guide message more readable.
 2.3. **ANALYZE** if the message is clear, easy to understand, cannot lead to misunderstandings

3. **Diff application:**
 3.1. **APPLY** changes to the selected files and generated todo message using the `patch` tool 
 3.2. **REPEAT** patch tool call if you see any error or you think that the generated patch does not fix the problem.

4. **Completion:**
4.1. **WHEN** you are sure that the generated diff solves the problem, **SEND** a separate message containing only the word: `{DONE_MESSAGE}`.

###What Not To Do!###
- DECIDE NOT TO FOLLOW THE PLAN ABOVE
- DO NOT REPEAT YOURSELF
- DO NOT ASK A TOOL WITH THE SAME ARGUMENTS TWICE!
- NEVER ADD EXTRA ARGUMENTS TO TOOLS.
- NEVER GUESS FILE CONTENTS WITHOUT TOOL OUTPUT.
- NEVER GENERATE PATCHES OR CHANGE CODE MANUALLY, USE PATCH TOOL"""


def print_step(step_n: int):
    console = Console()
    console.print(f"\n\n\n[bold]{'-' * 90}[/bold ]")
    console.print(f"[bold]{'-' * 90}[/bold ]")
    console.print(f"[bold]{'-' * 40}  STEP {step_n}  {'-' * 40}[/bold ]")
    console.print(f"[bold]{'-' * 90}[/bold ]")
    console.print(f"[bold]{'-' * 90}[/bold ]")


def print_messages(messages: List[Message]):
    def _is_tool_call(m: Message) -> bool:
        return m.tool_calls is not None and len(m.tool_calls) > 0

    console = Console()
    role_to_string = {
        "system": "[bold red]SYSTEM:[/bold red]",
        "assistant": "[bold red]ASSISTANT:[/bold red]",
        "user": "[bold red]USER:[/bold red]",
        "tool": "[bold red]TOOL ANSWER id={uid}:[/bold red]",
    }
    for m in messages:
        if m.role in role_to_string and m.content is not None:
            content = Markdown(m.content)
            header = role_to_string[m.role]
            if m.role == "tool":
                header = header.format(uid=m.tool_call_id[:20])
            console.print(header)
            console.print(content)
            console.print("")
        elif m.role == "context_file":
            message = "[bold red]CONTEXT FILE:[/bold red]\n"
            for file in json.loads(m.content):
                message += f"{file['file_name']}:{file['line1']}-{file['line2']}, len={len(file['file_content'])}\n"
            console.print(message)
        elif m.role == "diff":
            message = "[bold red]DIFF:[/bold red]\n"
            for chunk in json.loads(m.content):
                message += f"{chunk['file_name']}:{chunk['line1']}-{chunk['line2']}\n"
                if len(chunk["lines_add"]) > 0:
                    lines = [f"+{line}" for line in chunk['lines_add'].splitlines()]
                    lines = "\n".join(lines)
                    message += f"[bold green]{lines}[/bold green]\n"
                if len(chunk["lines_remove"]) > 0:
                    lines = [f"-{line}" for line in chunk['lines_remove'].splitlines()]
                    lines = "\n".join(lines)
                    message += f"[bold red]{lines}[/bold red]\n"
            console.print(message)
        if _is_tool_call(m):
            message = "[bold red]TOOL CALLS:[/bold red]\n"
            for tool_call in m.tool_calls:
                message += f"[bold]{tool_call.function.name}[/bold]({tool_call.function.arguments}) [id={tool_call.id[:20]}]\n"
            console.print(message)


class ProducePatchStep(Step):
    def __init__(self, attempts: int, files: List[str], *args, **kwargs):
        super().__init__(*args, **kwargs)
        self._attempts = attempts
        self._files = files

    @property
    def _tools(self) -> Set[str]:
        return {
            "tree",
            "definition",
            "references",
            "patch"
        }

    async def _patch_generate(self, repo_path: Path, messages: List[Message]):
        diff_messages = [json.loads(m.content) for m in messages if m.role == "diff"]
        all_filenames = set([diff["file_name"] for m in diff_messages for diff in m])
        applied_diffs = []
        for diff in diff_messages[::-1]:
            seen_filenames = set()
            for filename in map(lambda x: x["file_name"], diff):
                if filename in all_filenames:
                    seen_filenames.add(filename)
            if len(seen_filenames) > 0:
                await chat_client.diff_apply(self._base_url, chunks=diff, apply=[True] * len(diff))
                applied_diffs += diff
                all_filenames = all_filenames - seen_filenames

        if len(applied_diffs) > 0:
            result = subprocess.check_output(["git", "--no-pager", "diff"], cwd=str(repo_path.absolute())).decode()
            await chat_client.diff_apply(self._base_url, chunks=applied_diffs, apply=[False] * len(applied_diffs))
            Console().print(f"[bold red]FINAL DIFF:[/bold red]\n```\n{result}\n```")
            return result
        else:
            raise RuntimeError("there is no patch generated")

    async def _single_step(self, message: str, repo_path: Path) -> str:
        def _is_done(m: Message) -> bool:
            return m.role == "assistant" and m.content and m.content[-len(DONE_MESSAGE):] == DONE_MESSAGE

        problem = f"Problem statement:\n```\n{message}\n```\n\nList of files:\n```\n{self._files}\n```"
        messages = [
            chat_client.Message(role="system", content=f"{SYSTEM_MESSAGE}\n\n{problem}"),
        ]
        cursor = 0
        for step_n in range(self._max_depth):
            try:
                messages = await self._query(messages, verbose=False)
            except Exception as e:
                raise e
            try:
                print_step(step_n)
                print_messages(messages[cursor:])
            except Exception as e:
                raise e
            cursor = len(messages)
            if _is_done(messages[-1]):
                break
        return await self._patch_generate(repo_path, messages)

    async def process(self, task: str, repo_path: Path, **kwargs) -> List[str]:
        results = []
        for attempt_n in range(self._attempts):
            print(f"Attempt {attempt_n}")
            try:
                results.append(await self._single_step(task, repo_path))
            except Exception as e:
                print(f"attempt {attempt_n} is failed: {e}")
                continue
        if not results:
            raise RuntimeError(f"can't produce result with {self._attempts} attempts")
        return results
