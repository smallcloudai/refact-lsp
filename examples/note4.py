import argparse
import asyncio

from datetime import datetime
from refact import chat_client


MODEL = "llama3/8b/instruct"  # normal working of this model requires at least 8k context size
# MODEL = "gpt-4o"
# MODEL = "gpt-3.5-turbo"    # $0.50


TOOLS_TURN_ON = {"definition", "compile"}
DUMP_PREFIX = datetime.now().strftime("%Y%m%d-%H%M%S")


SYSTEM_PROMPT = """
You need to actively search for the answer yourself, don't ask the user to do anything. The answer is most likely
in the files and databases accessible using tool calls, not on the internet.

When responding to a query, first provide a very brief explanation of your plan to use tools in parallel to answer
the question, and then make several tool calls to gather more details.

Minimize the number of steps, call up to 5 tools in parallel when exploring. Use only one tool when executing.
Do not use tools that are not available.

BEGIN DEMONSTRATION

Example 1

User: "What is the weather like today in Paris and London?"
Assistant: "Must be sunny in Paris and foggy in London."
User: "don't hallucinate, use the tools"
Assistant: "Sorry for the confusion, you are right, weather is real-time, and my best shot is to use the weather tool. I will use 2 calls in parallel." <functioncall> [ { "name": "weather", "arguments": { "city": "London" } }, { "name": "weather", "arguments": { "city": "Paris" } } ] </functioncall>


Example 2

User: "What is MyClass"
Assistant: "Let me find it first." <functioncall>[{"name":"ls","arguments":"{\"file\":\".\"}"}]</functioncall>
Tool: folder1, folder2, folder3
Assistant: "I see 3 folders, will make 3 calls in parallel to check what's inside." <functioncall>[{"name":"ls","arguments":"{\"file\":\"folder1\"}"},{"name":"ls","arguments":"{\"file\":\"folder2\"}"},{"name":"ls","arguments":"{\"file\":\"folder3\"}"}]</functioncall>
Tool: ...
Tool: ...
Tool: ...
Assistant: "I give up, I can't find a file relevant for MyClass 😕"
User: "Look, it's my_class.cpp"
Assistant: "Sorry for the confusion, there is in fact a file named `my_class.cpp` in `folder2` that must be relevant for MyClass." <functioncall>[{"name":"cat","arguments":"{\"file\":\"folder2/my_class.cpp\"}"}]</functioncall>
Tool: ...
Assistant: "MyClass does this and this"

END DEMONSTRATION

Remember: explain your plan briefly before calling the tools in parallel.

IT IS FORBIDDEN TO JUST CALL TOOLS WITHOUT EXPLAINING. EXPLAIN FIRST!
"""


async def do_all():
    parser = argparse.ArgumentParser()
    parser.add_argument('--user', type=str, default="Explain what Frog is", help='User message')
    parser.add_argument('--depth', type=int, default=3, help='Depth of the assistant')
    parser.add_argument('--stream', action='store_true', help='Stream messages')
    args = parser.parse_args()

    messages = [
        chat_client.Message(role="system", content=SYSTEM_PROMPT),
        chat_client.Message(role="user", content=args.user),
    ]

    for step_n in range(args.depth):
        print("-" * 40 + f" step {step_n} " + "-" * 40)

        tools = await chat_client.tools_fetch_and_filter(base_url="http://127.0.0.1:8001/v1", tools_turn_on=TOOLS_TURN_ON)
        assistant_choices = await chat_client.ask_using_http(
            "http://127.0.0.1:8001/v1",
            messages,
            1,
            MODEL,
            tools=tools,
            verbose=True,
            temperature=0.6,
            stream=args.stream,
            max_tokens=2048,
        )

        messages = assistant_choices[0]
        with open(f"note_logs/{DUMP_PREFIX}.json", "w") as f:
            json_data = [msg.model_dump_json(indent=4) for msg in messages]
            f.write("[\n" + ",\n".join(json_data) + "\n]")
            f.write("\n")
        if not messages[-1].tool_calls:
            break


if __name__ == "__main__":
    asyncio.run(do_all())
