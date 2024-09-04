import json, random
import asyncio
from refact import chat_client
from pygments import highlight
from pygments.lexers import PythonLexer
from pygments.formatters import TerminalFormatter


def generate_tool_call(tool_name, tool_arguments):
    random_hex = ''.join(random.choices('0123456789abcdef', k=6))
    tool_call = {
        "id": f"{tool_name}_{random_hex}",
        "function": {
            "arguments": json.dumps(tool_arguments),
            "name": tool_name
        },
        "type": "function"
    }
    return tool_call


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


async def test_references(symbol: str, expected_references) -> None:

    initial_messages = [
        chat_client.Message(role="user", content=f"Call references() for {symbol}"),
        chat_client.Message(role="assistant", content="Alright, here we go", tool_calls=[generate_tool_call("references", {"symbol": symbol})]),
    ]

    # Act
    assistant_choices = await ask_chat(initial_messages)

    # Assert
    response_messages = assistant_choices[0][2:]

    tool_call_message = None
    for msg in response_messages:
        if msg.role == "tool" and tool_call_message is None:
            tool_call_message = msg

    assert tool_call_message is not None, "No tool called"
    assert "reference" in tool_call_message.tool_call_id, "It should call references tool, called: " + tool_call_message.tool_call_id
    assert "Found" in tool_call_message.content, "It should find references " + tool_call_message.content
    assert "references in the workspace" in tool_call_message.content, "It should find references " + tool_call_message.content

    for expected_reference in expected_references:
        assert tool_call_message.content.count(expected_reference["filename"]) >= expected_reference["count"], "It should find at least " + str(expected_reference["count"]) + " references in " + expected_reference["filename"]

    print("PASS: References test")


async def test_definition(function_name: str, function_full_definition: str, body_fragment: str) -> None:
    # Arrange
    initial_messages = [
        chat_client.Message(role="user", content=f"Call definition() for {function_name}"),
        chat_client.Message(role="assistant", content="Alright, here we go", tool_calls=[generate_tool_call("definition", {"symbol": function_name})]),
    ]

    # Act
    assistant_choices = await ask_chat(initial_messages)

    # Assert
    response_messages = assistant_choices[0][2:]

    tool_call_message = None
    context_file_message = None
    for msg in response_messages:
        if msg.role == "tool" and tool_call_message is None:
            tool_call_message = msg
        if msg.role == "context_file" and context_file_message is None:
            context_file_message = msg

    assert tool_call_message is not None, "No tool called"
    assert "definition" in tool_call_message.tool_call_id, "It should call definition tool, called: " + tool_call_message.tool_call_id
    assert function_full_definition in tool_call_message.content, "It should find the function definition: " + tool_call_message.content

    assert context_file_message is not None, "No context file"
    assert "def " + function_name in context_file_message.content, "Context file should contain function definition: " + context_file_message.content
    assert body_fragment in context_file_message.content, "Body of the function should be on the context file: " + context_file_message.content
    assert "..." in context_file_message.content, "It should not give entire file: " + context_file_message.content

    context_files = json.loads(context_file_message.content)
    for fdict in context_files:
        hl = highlight(fdict["file_content"], PythonLexer(), TerminalFormatter())
        print(hl)

    print("PASS: Definition test")


if __name__ == '__main__':
    asyncio.run(test_definition(
        function_name="bounce_off_banks", 
        function_full_definition="Frog::bounce_off_banks", 
        body_fragment="self.vy = -np.abs(self.vy)")
    )
    asyncio.run(test_definition(
        function_name="draw_hello_frog", 
        function_full_definition="draw_hello_frog", 
        body_fragment="text_rect = text.get_rect()")
    )
    asyncio.run(test_references(symbol="jump", expected_references=[
        {"filename": "emergency_frog_situation/holiday.py", "count": 8}, 
        {"filename": "emergency_frog_situation/jump_to_conclusions.py", "count": 1}, 
        {"filename": "emergency_frog_situation/set_as_avatar.py", "count": 1}, 
        {"filename": "emergency_frog_situation/work_day.py", "count": 1}, 
    ]))
    asyncio.run(test_references(symbol="bounce_off_banks", expected_references=[
        {"filename": "emergency_frog_situation/frog.py", "count": 1},
    ]))
    asyncio.run(test_references(symbol="draw_hello_frog", expected_references=[
        {"filename": "emergency_frog_situation/jump_to_conclusions.py", "count": 1},
    ]))

