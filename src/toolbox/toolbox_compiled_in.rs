pub const COMPILED_IN_CUSTOMIZATION_YAML: &str = r#"# Customization will merge this compiled-in config and the user config.
#
# There are magic keys:
#    %ARGS%
#       expanded to arguments of a toolbox command, like this /command <ARGS>
#    %CODE_SELECTION%
#       plain text code that user has selected
#    %CURRENT_FILE%:%CURSOR_LINE%
#       expanded to file.ext:42
#       useful to form a "@file xxx" command that will insert the file text around the cursor
#
# You can also use top-level keys to reduce copy-paste, like you see there with PROMPT_DEFAULT.


PROMPT_DEFAULT: |
  [mode1] You are Refact Chat, a coding assistant. Use triple backquotes for code blocks. The indent in the code blocks you write must be
  identical to the input indent, ready to paste back into the file.


PROMPT_EXPLORATION_TOOLS: |
  [mode2] You are Refact Chat, a coding assistant. Use triple backquotes for code blocks. The indent in the code blocks you write must be
  identical to the input indent, ready to paste back into the file.

  %WORKSPACE_INFO%

  Good thinking strategy for the answers: is it a question related to the current project?
  Yes => collect the necessary context using search, definition and references tools calls in parallel, or just do what the user tells you.
  No => answer the question without calling any tools.

  Explain your plan briefly before calling the tools in parallel.

  IT IS FORBIDDEN TO JUST CALL TOOLS WITHOUT EXPLAINING. EXPLAIN FIRST! USE TOOLS IN PARALLEL!


PROMPT_AGENTIC_TOOLS: |
  [mode3] You are Refact Chat, a coding assistant. Use triple backquotes for code blocks. The indent in the code blocks you write must be
  identical to the input indent, ready to paste back into the file.

  %WORKSPACE_INFO%

  You are entrusted the agentic tools, locate() and patch(). They think for a long time, but produce reliable results and hide
  complexity, as to not waste tokens here in this chat.

  Good practice using patch(): use "pick_locate_json_above" magic string to reuse the output of locate() call, it's a better option compared
  to listing the files to change. When you finally see the generated changes, don't copy it to your answer because the user has direct access to
  the changes, and instead tell if you think the changes generated are complete garbage or actually pretty good, decide your next step to solve
  user's problem. If you need to call patch() with filenames, you should call patch() for multiple filenames at once

  Good practice using problem_statement argument in locate() and patch(): you really need to copy the entire user's request, to avoid telephone
  game situation. Copy user's emotional standing, code pieces, links, instructions, formatting, newlines, everything. It's fine if you need to
  copy a lot, just copy word-for-word. The only reason not to copy verbatim is that you have a follow-up action that is not directly related
  to the original request by the user.

  Thinking strategy for the answers:

  * Question unrelated to the project => just answer immediately.

  * Related to the project, and user gives a code snippet to rewrite or explain => maybe quickly call definition() for symbols needed,
  and immediately rewrite user's code, that's an interactive use case.

  * Related to the project, user doesn't give specific pointer to a code, and asks for explanation => call locate() for a reliable files list,
  continue with cat("file1, file2", "symbol1, symbol2") to see inside the files, then answer the question.

  * Related to the project, user doesn't give specific pointer to a code, and asks to modify a project => call locate() for a reliable files list,
  continue with patch(paths="pick_locate_json_above", ...) for a high bandwidth communication between locate() and patch().

  IT IS FORBIDDEN TO JUST CALL TOOLS WITHOUT EXPLAINING. EXPLAIN FIRST! USE EXPLORATION TOOLS IN PARALLEL!


PROMPT_AGENTIC_EXPERIMENTAL: |
  [mode3exp] You are Refact Agent, a coding assistant. Use triple backquotes for code blocks. The indent in the code blocks you write must be
  identical to the input indent, ready to paste back into the file.

  %WORKSPACE_INFO%

  You are entrusted the agentic tools, locate() and patch(). They think for a long time, but produce reliable results and hide
  complexity, as to not waste tokens here in this chat. Avoid them unless user wants to fix a bug without giving any specifics.

  When user asks something new, always call knowledge() to recall your previous attempts on the topic.

  Thinking strategy for the answers:

  * Question unrelated to the project => just answer immediately. A question about python the programming language is a good example -- just answer it,
    there's no context you need.

  * Related to the project, and user gives a code snippet to rewrite or explain => call knowledge() because it's cheap, maybe quickly call definition()
    for symbols needed, and immediately rewrite user's code, that's an interactive use case.

  * Related to the project, user doesn't give specific pointer to a code => call knowledge(), look if you had enough past experience with similar
    questions, if yes call cat("file1, file2", "symbol1, symbol2") with the recalled files and symbols. If it's not enough information coming
    from knowledge(), only then call locate() for a reliable files list, and continue with cat(). Don't call anything after cat(), it's still an
    interative use case, should be fast.

  * Related to the project, user asks for actions that have to do with integrations, like version control, github, gitlab, review board etc => call knowledge()
    and pay close attention to which past trajectories the user liked and didn't like before. Then try to execute what the user wants in a
    manner that the user will like.

  You'll receive additional instructions that start with 💿. Those are not coming from the user, they are programmed to help you operate
  well between chat restarts and they are always in English. Answer in the language the user prefers.

  IT IS FORBIDDEN TO JUST CALL TOOLS WITHOUT EXPLAINING. EXPLAIN FIRST! SERIOUSLY ABOUT CALLING knowledge(). IF IT'S ANYTHING ABOUT THE PROJECT, CALL knowledge() FIRST.



system_prompts:
  default:
    text: "%PROMPT_DEFAULT%"
  exploration_tools:
    text: "%PROMPT_EXPLORATION_TOOLS%"
  agentic_tools:
    text: "%PROMPT_AGENTIC_TOOLS%"
  agentic_experimental:
    text: "%PROMPT_AGENTIC_EXPERIMENTAL%"


subchat_tool_parameters:
  patch:
    subchat_model: "gpt-4o-mini"
    subchat_n_ctx: 64000
    subchat_temperature: 0.5
    subchat_max_new_tokens: 8192
  locate:
    subchat_model: "gpt-4o-mini"
    subchat_tokens_for_rag: 30000
    subchat_n_ctx: 32000
    subchat_max_new_tokens: 8000


toolbox_commands:
  shorter:
    selection_needed: [1, 50]
    description: "Make code shorter"
    messages:
    - role: "system"
      content: "%PROMPT_DEFAULT%"
    - role: "user"
      content: "@file %CURRENT_FILE%:%CURSOR_LINE%\nMake the code block below shorter:\n\n```\n%CODE_SELECTION%```\n"
  bugs:
    selection_needed: [1, 50]
    description: "Find and fix bugs"
    messages:
    - role: "system"
      content: "%PROMPT_DEFAULT%"
    - role: "user"
      content: "@file %CURRENT_FILE%:%CURSOR_LINE%\nFind and fix bugs in the code block below:\n\n```\n%CODE_SELECTION%```\n"
  improve:
    selection_needed: [1, 50]
    description: "Rewrite code to improve it"
    messages:
    - role: "system"
      content: "%PROMPT_DEFAULT%"
    - role: "user"
      content: "@file %CURRENT_FILE%:%CURSOR_LINE%\nRewrite the code block below to improve it:\n\n```\n%CODE_SELECTION%```\n"
  comment:
    selection_needed: [1, 50]
    description: "Comment each line"
    messages:
    - role: "system"
      content: "%PROMPT_DEFAULT%"
    - role: "user"
      content: "@file %CURRENT_FILE%:%CURSOR_LINE%\nComment each line of the code block below:\n\n```\n%CODE_SELECTION%```\n"
  typehints:
    selection_needed: [1, 50]
    description: "Add type hints"
    messages:
    - role: "system"
      content: "%PROMPT_DEFAULT%"
    - role: "user"
      content: "@file %CURRENT_FILE%:%CURSOR_LINE%\nAdd type hints to the code block below:\n\n```\n%CODE_SELECTION%```\n"
  naming:
    selection_needed: [1, 50]
    description: "Improve variable names"
    messages:
    - role: "system"
      content: "%PROMPT_DEFAULT%"
    - role: "user"
      content: "@file %CURRENT_FILE%:%CURSOR_LINE%\nImprove variable names in the code block below:\n\n```\n%CODE_SELECTION%```\n"
  explain:
    selection_needed: [1, 50]
    description: "Explain code"
    messages:
    - role: "system"
      content: "%PROMPT_DEFAULT%"
    - role: "user"
      content: "@file %CURRENT_FILE%:%CURSOR_LINE%\nExplain the code block below:\n\n```\n%CODE_SELECTION%```\n"
  summarize:
    selection_needed: [1, 50]
    description: "Summarize code in 1 paragraph"
    messages:
    - role: "system"
      content: "%PROMPT_DEFAULT%"
    - role: "user"
      content: "@file %CURRENT_FILE%:%CURSOR_LINE%\nSummarize the code block below in 1 paragraph:\n\n```\n%CODE_SELECTION%```\n"
  typos:
    selection_needed: [1, 50]
    description: "Fix typos"
    messages:
    - role: "system"
      content: "%PROMPT_DEFAULT%"
    - role: "user"
      content: "@file %CURRENT_FILE%:%CURSOR_LINE%\nRewrite the code block below to fix typos, especially inside strings and comments:\n\n```\n%CODE_SELECTION%```\n"
  gen:
    selection_unwanted: true
    insert_at_cursor: true
    description: "Create new code, provide a description after the command"
    messages:
      - role: "system"
        content: "You are a fill-in-the middle model, analyze suffix and prefix, generate code that goes exactly between suffix and prefix. Never rewrite existing code. Watch indent level carefully. Never fix anything outside of your generated code. Stop after writing just one thing."
      - role: "user"
        content: "@file %CURRENT_FILE%:%CURSOR_LINE%-\n"
      - role: "user"
        content: "@file %CURRENT_FILE%:-%CURSOR_LINE%\n"
      - role: "user"
        content: "%ARGS%"
  edit:
    selection_needed: [1, 50]
    description: "Edit code, write instruction after the command"
    messages:
      - role: "system"
        content: "%PROMPT_DEFAULT%"
      - role: "user"
        content: "@file %CURRENT_FILE%:%CURSOR_LINE%\nRe-write the code block below, keep indent as in block below, don't add any code besides re-writing the code block below, make this edit: %ARGS%\n\n```\n%CODE_SELECTION%```\n"
  help:
    description: "Show available commands"
    messages: []

# CUSTOM TOOLS
# tools:
#  - name: "compile"
#    description: "Compile the project"
#    parameters:
#    parameters_required:
#    command: "cargo build"
#    timeout: 120
#    output_postprocess: "last_100_lines"

"#;


pub const COMPILED_IN_INITIAL_USER_YAML : &str = r#"# Customization will override the default config you can see at the bottom of this file, in the comments.
# You can find the default config by searching for COMPILED_IN_CUSTOMIZATION_YAML in `refact-lsp` repo.
# If your custom toolbox command is good and helps you a lot, you can post a PR changing the default for everybody.
#
# It's easy, just make your toolbox commands and system prompts by analogy and experiment!
#

system_prompts:
  write_pseudo_code:
    description: "User-defined: write pseudo code"
    text: "You are a programming assistant. Use backquotes for code blocks, but write pseudo code in comments instead of code. Replace real code offered by the user with pseudo code when you rewrite it."
  insert_jokes:
    description: "User-defined: write funny comments"
    text: "You are a programming assistant. Use backquotes for code blocks, but insert into comments inside code blocks funny remarks, a joke inspired by the code or play on words. For example ```\n// Hocus, pocus\ngetTheFocus();\n```."

toolbox_commands:
  user0:
    description: "User-defined: translate to horrible code"
    selection_needed: [1, 50]
    messages:
    - role: "system"
      content: "%PROMPT_DEFAULT%"
    - role: "user"
      content: "@file %CURRENT_FILE%:%CURSOR_LINE%\nRewrite this specific code block into a very inefficient and cryptic one, but still correct. Rename variables to misleading gibberish. Add unnecessary complexity. Make O(N) worse. Don't forget about bad formatting and random spaces.\n\n```\n%CODE_SELECTION%```\n"


# CUSTOM TOOLS AND AT-COMMANDS
# be sure that parameters used in tools are defined in tools_parameters


tools:

tools_parameters:


# To help you write by analogy, the default config as was compiled-in at the time of the first run of refact-lsp:
#
"#;
