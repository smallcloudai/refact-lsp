
// todo: ask it to specify symbols to change / to context
pub const CAT_REDUCE_SYMBOLS_PROMPT: &str = r###"
Read slowly and carefully the problem text one more time before you start:

{USER_QUERY}

Proposed symbols:
{PROPOSED_SYMBOLS}

TODO:
1. analyse thoroughly the problem statement;
2. analyse thoroughly context given (skeletonized files from the previous message);
3. read proposed symbols;
4. according to the problem statement pick ONLY those symbols from proposed symbols list that:
  a) provide the context important to the problem;
  b) need to be changed to solve the problem.
5. return the results in a format specified below in a json format, explain nothing.

Format you must obey:
[
    "variable1",
    "function11",
    "ClassName111"
]
"###;

pub const CAT_REDUCE_TO_CHANGE_PROMPT: &str =  r###"
Read slowly and carefully the problem text one more time before you start:

{USER_QUERY}

TODO:
1. analyse thoroughly the problem statement;
2. read thoroughly the list of files;
3. pick among those files given pick those that you are absolutely 100% you need to change;
4. return the results in a format specified below in a json format, explain nothing, file_path must be an absolute path.

Format you must obey:
[
    "/a/b/c/file1.py",
    "a/d/e/file2.py"
]
"###;

pub const CAT_FILE_TO_CHANGE_PROMPT: &str = r###"
Read slowly and carefully the problem text one more time before you start:

{USER_QUERY}

In the previous message you were given a generous context -- skeletonized files.

TODO:
1. analyse thoroughly the problem statement;
2. analyse thoroughly context given (skeletonized files from the previous message);
3. among the files pick the ones you need to make changes in to solve the problem (according to the problem statement);
5. return the results in a format specified below in a json format, explain nothing, file_path must be an absolute path.

Format you must obey:
[
    "/a/b/c/file1.py",
    "a/d/e/file2.py"
]
"###;

pub const SUPERCAT_EXTRACT_SYMBOLS_PROMPT: &str = r###"
Read slowly and carefully the problem text one more time before you start:

{USER_QUERY}

1. analyse thoroughly the problem statement;
2. analyse thoroughly context given (skeletonized files from the previous message);
3. from the given context select all the symbols (functino names, classes names etc) that you find relevant to the problem (either give releavant context or need to be changed);
4. return found symbols in a json format, explain nothing.

Format you must obey:
[
    "variable1",
    "function11",
    "ClassName111"
]
"###;

pub const STRATEGY_TREE_PROMPT: &str = r###"
TODO:
1. analyse thoroughly the problem statement;
2. look thoroughly at the project tree given;
3. pick at least 10 files that will help you solving the problem (ones that give you the context and ones that shall be changed);
4. return chosen files in a json format, explain nothing.

Format you must obey:
[
    "file1.py",
    "file2.py"
]
"###;

pub const STRATEGY_DEF_REF_PROMPT: &str = r###"
TODO:
1. analyse thoroughly the problem statement;
2. from the problem statement pick up AST Symbols (classes, functions, types, variables etc) that are relevant to the problem;
3. return found symbols in a json format, explain nothing.

Format you must obey:
[
    "variable1",
    "function11",
    "ClassName111"
]
"###;

pub const LOCATE_SYSTEM_PROMPT: &str = r###"
You are a genius coding assistant named "Refact". You are known for your scruplousness and well thought-out code.
Listening to the user is what makes you the best.
"###;
