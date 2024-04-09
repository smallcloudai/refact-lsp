import os
import random

import requests
import termcolor

from pathlib import Path

# To test, run in a second console:
# target/debug/refact-lsp --address-url Refact --http-port 8001 --workspace-folder tests/emergency_frog_situation --ast --logs-stderr
# and wait for AST COMPLETE

REPO_PATH = Path(os.path.dirname(__file__)) / "emergency_frog_situation"
SPECIAL_TOKENS = ["<fim_prefix>", "<fim_suffix>", "<fim_middle>"]


def code_completion_prompt_with_rag(filename: Path):
    sample_code = filename.read_text()

    while True:
        try:
            lines = sample_code.split("\n")
            selected_line_idx = random.randint(0, len(lines) - 1)
            selected_char_idx = random.randint(0, len(lines[selected_line_idx]) - 1)
            selected_line = lines[selected_line_idx]
            prefix_selected, middle = selected_line[:selected_char_idx], selected_line[selected_char_idx:]
            prefix_lines, suffix_lines = [*lines[:selected_line_idx], prefix_selected], lines[selected_line_idx + 1:]
            break
        except:
            continue

    response = requests.post(
        url="http://127.0.0.1:8001/v1/code-completion-prompt",
        json={
            "inputs": {
                "sources": {str(filename): "\n".join(prefix_lines + suffix_lines)},
                "cursor": {
                    "file": str(filename),
                    "line": selected_line_idx,
                    "character": selected_char_idx,
                },
                "multiline": True
            },
            "use_ast": True,
        },
        headers={
            "Content-Type": "application/json",
        },
    )
    prompt = response.json()["prompt"]

    print(termcolor.colored(f"Prompt for {filename}:", "yellow"))
    for token in SPECIAL_TOKENS:
        prompt = prompt.replace(token, termcolor.colored(token, "blue"))
    prompt = prompt.replace("\n".join(prefix_lines), termcolor.colored("\n".join(prefix_lines), "yellow"))
    middle = termcolor.colored(middle, 'green')
    print(f"{prompt}{middle}\n\n")


if __name__ == "__main__":
    for filename in REPO_PATH.rglob("*.py"):
        code_completion_prompt_with_rag(filename)
