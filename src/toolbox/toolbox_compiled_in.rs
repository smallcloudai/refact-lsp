pub const COMPILED_IN_TOOLBOX_YAML : &str = r#"
# Toolbox will merge this compiled-in config and the user config.
#
# There are magic keys:
#    $ARGS
#       expanded to arguments of a command, like this /command <ARGS>
#    $CODE_SELECTION
#       plain text code that user has selected
#    $CODE_AROUND_CURSOR_JSON
#       Json that has the current file, possibly cut (if it's large) to have the cursor in
#       the middle. The json format is suitable to attach to a role="context_file" message
#    $CODE_INSERT_HERE_JSON
#       Json that has the current file, cursor position marked with "|INSERT-HERE|" in the text
#
SYSTEM_PROMPT: "You are a programming assistant. Use backquotes for code blocks, give links to documentation at the end of the response."
commands:
  shorter:
    selection_needed: [1, 50]
    description: "Make code shorter"
    messages:
      - ["system", "$SYSTEM_PROMPT"]
      - ["context_file", "$CODE_AROUND_CURSOR_JSON"]
      - ["user", "Make this specific code block shorter\n\n```\n$CODE_SELECTION```\n"]
  new:
    selection_unwanted: true
    insert_at_cursor: true
    description: "Create new code, provide a description after the command"
    messages:
      - ["system", "You are an expert in writing new clean code, repeat in one paragraph how did you understand the instructions. Code needs to fit in the context around |INSERT-HERE| mark. Write a single block of code in backquotes that exactly implements the description. Do nothing else. Don't fix imports. The indent must match |INSERT-HERE| mark."]
      - ["user", "@workspace $ARGS", "xxx"]
      - ["context_file", "$CODE_INSERT_HERE_JSON"]
      - ["user", "Generate new code according this description: $ARGS"]
  why:
    selection_needed: [1, 50]
    description: "Explain how this code fits into the project and why it exists"
    messages:
      - ["system", "$SYSTEM_PROMPT"]
      - ["context_file", "$WHY_PROMPT"]
"#;
