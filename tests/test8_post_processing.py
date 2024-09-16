import json, random, termcolor
import asyncio
from refact import chat_client
from pygments import highlight
from pygments.lexers import find_lexer_class_for_filename
from pygments.formatters import TerminalFormatter
from typing import Any, Dict


async def ask_chat(messages):
    tools_turn_on = {"definition", "references", "search", "cat"}
    tools = await chat_client.tools_fetch_and_filter(base_url="http://127.0.0.1:8001/v1", tools_turn_on=tools_turn_on)
    assistant_choices = await chat_client.ask_using_http(
        "http://127.0.0.1:8001/v1",
        messages,
        1,
        "gpt-4o-mini",
        tools=tools,
        verbose=False,
        temperature=0.3,
        stream=True,
        max_tokens=2048,
        only_deterministic_messages=True,
        postprocess_parameters={
            "take_floor": 50.0,
        }
    )
    return assistant_choices


def sort_out_messages(response_messages):
    tool_call_message = None
    context_file_message = None
    for msg in response_messages:
        print(f"role={msg.role!r}, content={msg.content[:30]!r}")
        if msg.role == "tool":
            assert tool_call_message is None
            tool_call_message = msg
        if msg.role == "context_file":
            assert context_file_message is None
            context_file_message = msg
    if tool_call_message is not None:
        print(termcolor.colored(tool_call_message.content, "blue"))
    if context_file_message is not None:
        context_files = json.loads(context_file_message.content)
        for fdict in context_files:
            lexer = find_lexer_class_for_filename(fdict["file_name"])()
            hl = highlight(fdict["file_content"], lexer, TerminalFormatter())
            print(hl.rstrip())
    return tool_call_message, context_file_message


async def test_tool_call(tool_name: str, symbol: str, this_number_of_lines_should_have: Dict[str, int] = None, should_present_in_context_file: str = None) -> None:
    print(f"\ntesting {tool_name}({symbol!r})")
    initial_messages = [
        chat_client.Message(role="user", content=f"Call {tool_name}() for {symbol}"),
        chat_client.Message(role="assistant", content="Alright, here we go", tool_calls=[chat_client.pretend_function_call(tool_name, {"symbol": symbol})]),
    ]
    assistant_choices = await ask_chat(initial_messages)
    tool_call_message, context_file_message = sort_out_messages(assistant_choices[0][2:])

    assert tool_call_message is not None, "No tool called"
    assert tool_name in tool_call_message.tool_call_id, f"It should call {tool_name} tool, called: " + tool_call_message.tool_call_id
    assert context_file_message, "no file_context, might be because take_floor is too high"
    assert should_present_in_context_file in context_file_message.content, f"'{should_present_in_context_file!r}' doesn't present in context_file"

    for substring, count in this_number_of_lines_should_have.items():
        print(f"Checking if tool has {count} of {substring!r}...")
        real = tool_call_message.content.count(substring)
        assert real == count, real
    assert "..." in context_file_message.content, "It should not give entire file"

    print(termcolor.colored("PASS", "green"))


if __name__ == '__main__':
    asyncio.run(test_tool_call(
        tool_name="definition",
        symbol="bounce_off_banks",
        this_number_of_lines_should_have={
            "Frog::bounce_off_banks": 1
        },
        should_present_in_context_file="self.vy = -np.abs(self.vy)"
    ))
    asyncio.run(test_tool_call(
        tool_name="definition",
        symbol="draw_hello_frog",
        this_number_of_lines_should_have={
            "jump_to_conclusions::draw_hello_frog": 1
        },
        should_present_in_context_file="text_rect = text.get_rect()"
    ))
    asyncio.run(test_tool_call(
        tool_name="references",
        symbol="cpp_goat_library::Animal::self_review",
        this_number_of_lines_should_have={
            "src/ast/alt_testsuite/cpp_goat_main.cpp": 1,
            "src/ast/alt_testsuite/cpp_goat_library.h": 3,
        },
        should_present_in_context_file="CosmicGoat f_local_goat",
    ))
    asyncio.run(test_tool_call(
        tool_name="references",
        symbol="Frog::jump",
        this_number_of_lines_should_have={
            # PYTHON PARSER BROKEN
            # "emergency_frog_situation/holiday.py": 8,
            "emergency_frog_situation/jump_to_conclusions.py": 1,
            # PYTHON PARSER BROKEN
            # "emergency_frog_situation/set_as_avatar.py": 1,
            # PYTHON PARSER BROKEN
            # "emergency_frog_situation/work_day.py": 1,
        },
        should_present_in_context_file="p.jump(W, H)"
    ))
