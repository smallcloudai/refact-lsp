pub const COMPILED_IN_TOOLBOX_YAML : &str = r#"
# Toolbox will merge this compiled-in config and the user config.
#
# There are magic keys:
#    %ARGS%
#       expanded to arguments of a command, like this /command <ARGS>
#    %CODE_SELECTION%
#       plain text code that user has selected
#    %CODE_AROUND_CURSOR_JSON%
#       Json that has the current file, possibly cut (if it's large) to have the cursor in
#       the middle. The json format is suitable to attach to a role="context_file" message
#    %CODE_INSERT_HERE_JSON%
#       Json that has the current file, cursor position marked with "|INSERT-HERE|" in the text
#

SYSTEM_PROMPT: "You are a programming assistant. Use backquotes for code blocks, give links to documentation at the end of the response."

SYSTEM_PROMPT_WHY: |
  Explain the purpose of the given code. Steps:
  In the [ORIGINAL_CODE_STEP] user will provide code surrounding the code snippet in question, and then the snippet itself will start with 🔥code and backquotes.
  In the [PROVIDE_COMMANDS_STEP] you need to ask for an extra context to completely understand the 🔥code and it's role in the project.
  Run several commands in a single message. Don't write any explanations on this step.
  Write the number of commands you plan to issue as a first line of your response, and then write all the commands.
  Ask for definitions of types used in the 🔥code.
  Ask for usages of the class or function defined in the 🔥code.
  Don't look up symbols you already have.
  Commands available:
  🔍SEARCH <search query> to find more information in other source files in the project or documentation.
  🔍FILE <path/file> to see whole file text.
  🔍DEFINITION <symbol>
  A example of command usage:
  3
  🔍SEARCH usages of function f
  🔍DEFINITION Type1
  🔍FILE repo1/test_file.cpp
  In the [GENERATE_DOCUMENTATION_STEP] you need to generate an explanation of the 🔥code.
  Answer questions "why it exists", "how does it fit into broader context". Don't explain line-by-line. Don't explain class data fields.
  Your response size should be one or two paragraphs.

commands:
  shorter:
    selection_needed: [1, 50]
    description: "Make code shorter"
    messages:
      - ["system", "%SYSTEM_PROMPT%"]
      - ["context_file", "%CODE_AROUND_CURSOR_JSON%"]
      - ["user", "Make this specific code block shorter\n\n```\n%CODE_SELECTION%```\n"]
  new:
    selection_unwanted: true
    insert_at_cursor: true
    description: "Create new code, provide a description after the command"
    messages:
      - ["system", "You are an expert in writing new clean code, repeat in one paragraph how did you understand the instructions. Code needs to fit in the context around |INSERT-HERE| mark. Write a single block of code in backquotes that exactly implements the description. Do nothing else. Don't fix imports. The indent must match |INSERT-HERE| mark."]
      - ["user", "@workspace %ARGS%", "xxx"]
      - ["context_file", "%CODE_INSERT_HERE_JSON%"]
      - ["user", "Generate new code according this description: %ARGS%"]
  why:
    selection_needed: [1, 50]
    description: "Explain how this code fits into the project and why it exists"
    messages:
      - ["system", "%SYSTEM_PROMPT_WHY%"]
      - ["context_file", "%WHY_PROMPT%"]
"#;
