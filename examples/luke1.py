import termcolor
import chat_with_at_command


my_prompt = """
Explain the purpose of the given code.

STEPS:

In the [ORIGINAL_CODE_STEP] user will provide code surrounding the code snippet in question, and then the snippet itself will start with 🔥code and backquotes.

In the [PROVIDE_COMMANDS_STEP] you need to ask for an extra context to completely understand the 🔥code and it's role in the project.
Run several commands in a single message. Don't write any explanations on this step.
Write the number of commands you plan to issue as a first line of your response,
and then write all the commands.
Commands available:

🔍SEARCH <search query> to find more information in other source files in the project or documentation.

🔍FILE <path/file> to see whole file text.

🔍DEFINITION <symbol>

Ask for definitions of types used in the 🔥code.
Ask for usages of the class or function defined in the 🔥code.
Don't look up symbols you already have.

A example of command usage:

3

🔍SEARCH usages of function f

🔍DEFINITION Type1

🔍FILE repo1/test_file.cpp

In the [GENERATE_DOCUMENTATION_STEP] you have to generate an explanation of the 🔥code.
Answer questions "why it exists", "how does it fit into broader context". Don't explain line-by-line. Don't explain class data fields.
Your response size should be one or two paragraphs.
"""

to_explain = """pub struct DeltaDeltaChatStreamer {
    pub delta1: String,
    pub delta2: String,
    pub finished: bool,
    pub stop_list: Vec<String>,
    pub role: String,
}
"""

initial_messages = [
{"role": "system", "content": my_prompt},
{"role": "user", "content":
    "[ORIGINAL_CODE_STEP]\n" +
    "@file /home/user/.refact/tmp/unpacked-files/refact-lsp/src/scratchpads/chat_utils_deltadelta.rs\n" +
    "Why this 🔥code exists:\n```\n[CODE]```\n".replace("[CODE]", to_explain) +
    "[PROVIDE_COMMANDS_STEP]\n"},
]


def rewrite_assistant_says_to_at_commands(ass):
    out = ""
    for s in ass.splitlines():
        s = s.strip()
        if not s:
            continue
        if s.startswith("🔍SEARCH"):
            out += "@workspace " + s[8:] + "\n"
        if s.startswith("🔍FILE"):
            out += "@file " + s[6:] + "\n"
        if s.startswith("🔍DEFINITION"):
            out += "@ast_definition " + s[12:] + "\n"
    return out


def dialog_turn(messages):
    for msgdict in messages:
        chat_with_at_command.msg_pretty_print(msgdict, normal_color="green")
    messages_back = chat_with_at_command.ask_chat(messages)
    for msgdict in messages_back:
        chat_with_at_command.msg_pretty_print(msgdict, normal_color="white")

    assistant_says = messages_back[-1]["content"]
    messages_without_last_user = messages[:-1]
    next_step_messages = messages_without_last_user + messages_back
    automated_new_user = rewrite_assistant_says_to_at_commands(assistant_says)
    if not automated_new_user:
        return next_step_messages, False
    automated_new_user += "[GENERATE_DOCUMENTATION_STEP]"
    next_step_messages.append({"role": "user", "content": automated_new_user})
    return next_step_messages, True


def do_all():
    messages = initial_messages.copy()
    for step in range(2):
        print("-"*40, "STEP%02d" % step, "-"*40)
        messages, need_automated_post = dialog_turn(messages)
        if not need_automated_post:
            break


do_all()

