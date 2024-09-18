import asyncio
import json
import sys
import argparse
import requests
import random
import termcolor
import aiohttp
from pydantic import BaseModel
from typing import Dict, Any, Optional, List, Union, Tuple

from prompt_toolkit import PromptSession, Application
from prompt_toolkit.patch_stdout import patch_stdout
from prompt_toolkit.key_binding import KeyBindings
from prompt_toolkit.completion import Completer, Completion
from prompt_toolkit.layout import Layout, CompletionsMenu, Float
from prompt_toolkit.layout.containers import HSplit, Window, FloatContainer
from prompt_toolkit.buffer import Buffer
from prompt_toolkit.layout.controls import BufferControl, FormattedTextControl
from prompt_toolkit.key_binding import KeyBindings
from prompt_toolkit.formatted_text import PygmentsTokens
from prompt_toolkit.widgets import TextArea

import refact.chat_client as chat_client
from refact.chat_client import Message, FunctionDict
from refact.printing import create_box, indent, print_header, get_terminal_width, tokens_len, Lines
from refact.status_bar import bottom_status_bar, update_vecdb_status_background_task
from refact.lsp_runner import LSPServerRunner


class CapsModel(BaseModel):
    n_ctx: int
    similar_models: List[str]
    supports_tools: bool


class Caps(BaseModel):
    cloud_name: str
    code_chat_models: Dict[str, CapsModel]
    code_chat_default_model: str
    embedding_model: str


async def fetch_caps(base_url: str) -> Caps:
    url = f"{base_url}/caps"
    async with aiohttp.ClientSession() as session:
        async with session.get(url) as response:
            if response.status == 200:
                data = await response.json()
                return Caps(**data)  # Parse the JSON data into the Caps model
            else:
                print(f"cannot fetch {url}\n{response.status}")
                return None


class CmdlineSettings:
    def __init__(self, caps, args):
        self.caps = caps
        self.model = args.model or caps.code_chat_default_model
        self.project_path = args.path_to_project

    def n_ctx(self):
        return self.caps.code_chat_models[self.model].n_ctx

settings = None


def find_tool_call(messages: List[Message], id: str) -> Optional[FunctionDict]:
    for message in messages:
        if message.tool_calls is None:
            continue
        for tool_call in message.tool_calls:
            if tool_call.id != id:
                continue
            return tool_call.function
    return None


def print_response(to_print: Union[str, List[Tuple[str, str]]]):
    if type(to_print) == str:
        response_box.text.append(("", to_print))
        app.invalidate()
        return

    for section in to_print:
        response_box.text.append(section)
    app.invalidate()


def print_lines(lines: Lines):
    global response_box
    
    for line in lines:
        number_of_children = len(hsplit.children)
        text_control = FormattedTextControl(text=PygmentsTokens(line))
        hsplit.children.insert(number_of_children - 1, Window(height=1, content=text_control))

    number_of_children = len(hsplit.children)
    response_box = FormattedTextControl(text=[])
    hsplit.children.insert(number_of_children - 1, Window(dont_extend_height=True, content=response_box))


def print_context_file(json_str: str):
    file = json.loads(json_str)[0]
    content = file["file_content"]
    file_name = file["file_name"]
    # line1 = file["line1"]
    # line2 = file["line2"]

    terminal_width = get_terminal_width()
    box = create_box(content, terminal_width - 4, max_height=20,
                     title=file_name, file_name=file_name)
    indented = indent(box, 2)
    print_response("\n")
    print_lines(indented)


streaming_messages = []
tools = []
lsp = None
response_box = None


def process_streaming_data(data):
    global streaming_messages
    if "choices" in data:
        choices = data['choices']
        delta = choices[0]['delta']
        content = delta['content']
        if content is None:
            finish_reason = choices[0]['finish_reason']
            if finish_reason == 'stop':
                print_response("\n")
            return
        if len(streaming_messages) == 0 or streaming_messages[-1].role != "assistant":
            print_response("\n  ")
            streaming_messages.append(
                Message(role="assistant", content=content))
        else:
            streaming_messages[-1].content += content

        content = content.replace("\n", "\n  ")
        print_response(content)
    elif "role" in data:
        role = data["role"]
        if role == "user":
            return

        content = data["content"]
        streaming_messages.append(Message(role=role, content=content))

        if role == "context_file":
            print_context_file(content)
            return

        terminal_width = get_terminal_width()
        box = create_box(content, terminal_width - 4, max_height=26)
        indented = indent(box, 2)
        tool_call_id = data["tool_call_id"]
        print_response("\n")
        function = find_tool_call(streaming_messages, tool_call_id)
        if function is not None:
            print_response(f"  {function.name}({function.arguments})")
        print_lines(indented)


async def ask_chat(model):
    global streaming_messages
    N = 1
    for step_n in range(4):
        def callback(data):
            process_streaming_data(data)

        messages = list(streaming_messages)

        new_messages = await chat_client.ask_using_http(
            lsp.base_url,
            messages,
            N,
            model,
            tools=tools,
            verbose=False,
            temperature=0.3,
            stream=True,
            max_tokens=2048,
            only_deterministic_messages=False,
            callback=callback,
        )
        streaming_messages = new_messages[0]

        if not streaming_messages[-1].tool_calls:
            break


async def answer_question_in_arguments(session: PromptSession, settings, arg_question):
    global streaming_messages
    streaming_messages.append(Message(role="user", content=arg_question))
    await ask_chat(settings.model)


tips_of_the_day = '''
Ask anything: "does this project have SQL in it?"
Ask anything: "summarize README"
Refact Agent is essentially its tools, ask: "what tools do you have?"
'''.strip().split('\n')


async def welcome_message(settings: CmdlineSettings, tip: str):
    text = f"""
~/.cache/refact/bring-your-own-key.yaml -- set up models you want to use
~/.cache/refact/integrations.yaml       -- set up github, jira, make, gdb, and other tools, including which actions require confirmation
~/.cache/refact/privacy.yaml            -- which files should never leave your computer
Project path: {settings.project_path}     Model: {settings.model} context={settings.n_ctx()}
To exit, type 'exit' or Ctrl+D. {tip}.
"""
    print(termcolor.colored(text.strip(), "white", None, ["dark"]))

kb = KeyBindings()


@kb.add('escape', 'enter')
def _(event):
    event.current_buffer.insert_text('\n')


@kb.add('enter')
def _(event):
    event.current_buffer.validate_and_handle()


@kb.add('c-c')
def _(event):
    event.current_buffer.reset()


@kb.add('c-d')
def exit_(event):
    event.app.exit()


class ToolsCompleter(Completer):
    def __init__(self):
        pass

    def get_completions(self, document, complete_event):
        text = document.text
        position = document.cursor_position
        response = get_at_command_completion(lsp.base_url, text, position)

        completions = response["completions"]
        replace = response["replace"]
        for completion in completions:
            yield Completion(completion, start_position=-position + replace[0])


def get_at_command_completion(base_url: str, query: str, cursor_pos: int) -> Any:
    url = f"{base_url}/at-command-completion"
    post_me = {
        "query": query,
        "cursor": cursor_pos,
        "top_n": 6,
    }
    result = requests.post(url, json=post_me)
    return result.json()


def on_submit(buffer):
    global response_box

    user_input = buffer.text
    if user_input.strip() == '':
        return
    if user_input.lower() in ('exit', 'quit'):
        app.exit()
        return

    number_of_children = len(hsplit.children)
    text_control = FormattedTextControl(text=[])
    response_box = text_control
    hsplit.children.insert(number_of_children - 1, Window(dont_extend_height=True, content=text_control))

    streaming_messages.append(Message(role="user", content=user_input))

    async def asyncfunc():
        await ask_chat(settings.model)

    loop = asyncio.get_event_loop()
    loop.create_task(asyncfunc())



async def chat_main():
    global tools
    global streaming_messages
    global lsp
    global settings
    streaming_messages = []

    args = sys.argv[1:]
    if '--' in args:
        split_index = args.index('--')
        before_minus_minus = args[:split_index]
        after_minus_minus = args[split_index + 1:]
    else:
        before_minus_minus = args
        after_minus_minus = []

    parser = argparse.ArgumentParser(
        description='Refact Agent access using command-line interface')
    parser.add_argument('path_to_project', type=str, nargs='?',
                        help="Path to the project", default=None)
    parser.add_argument('--model', type=str, help="Specify the model to use")
    parser.add_argument('question', nargs=argparse.REMAINDER,
                        help="You can continue your question in the command line after --")
    args = parser.parse_args(before_minus_minus)

    arg_question = " ".join(after_minus_minus)

    lsp = LSPServerRunner("./", "logs.txt", True, True,
                          wait_for_ast=False,
                          wait_for_vecdb=False,
                          verbose=False
                          )

    async with lsp:
        tools_turn_on = {"definition", "references", "file",
                         "search", "cat", "tree", "web"}

        session = PromptSession()
        asyncio.create_task(update_vecdb_status_background_task())

        tools = await chat_client.tools_fetch_and_filter(base_url=lsp.base_url, tools_turn_on=tools_turn_on)
        tool_completer = ToolsCompleter()

        caps = await fetch_caps(lsp.base_url)

        settings = CmdlineSettings(caps, args)
        if settings.model not in caps.code_chat_models:
            print(f"model {settings.model} is unknown, pick one of {
                  sorted(caps.code_chat_models.keys())}")
            return

        session = PromptSession()

        if arg_question:
            print(arg_question)
            await answer_question_in_arguments(session, settings, arg_question)
            return

        await welcome_message(settings, random.choice(tips_of_the_day))
        asyncio.create_task(update_vecdb_status_background_task())

        result = await app.run_async()

        with patch_stdout():
            while True:
                try:
                    user_input = await session.prompt_async(
                        "Chat> ",
                        key_bindings=kb,
                        bottom_toolbar=bottom_status_bar,
                        multiline=True,
                        refresh_interval=0.1,
                        completer=tool_completer,
                    )
                    if user_input.strip() == '':
                        continue
                    if user_input.lower() in ('exit', 'quit'):
                        break

                    streaming_messages.append(
                        Message(role="user", content=user_input))

                    await ask_chat(settings.model)
                    print()
                except EOFError:
                    print("\nclean exit")
                    break

tool_completer = ToolsCompleter()
text_area = TextArea(height=10, multiline=True, accept_handler=on_submit, completer=tool_completer)
hsplit = HSplit([FloatContainer(content=text_area, floats=[
    Float(xcursor=True, ycursor=True, content=CompletionsMenu())]
)])
layout = Layout(hsplit)
app = Application(key_bindings=kb, layout=layout)


def cmdline_main():
    asyncio.run(chat_main())


if __name__ in '__main__':
    asyncio.run(chat_main())
