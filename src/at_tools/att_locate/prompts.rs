
pub const SUPERCAT_REDUCE_TO_CHANGE: &str =  r###"
Read slowly and carefully the problem text one more time before you start:

{USER_QUERY}

TODO:
1. analyse thoroughly the problem statement;
2. read thoroughly the list of files;
3. pick among those files given pick those that you are absolutely 100% you need to change;

Format you must obey:
[
    {
        "file_path": "/a/b/c/file.py",
        "reason": "to_change",
        "description": "contains class MyClass, body of which needs to be changed."
    },
]

file_path must be an absolute path.
format you return must be a valid JSON, explain nothing, don't use any quotes or backticks.

"###;

pub const SUPERCAT_DECIDER_PROMPT: &str = r###"
Read slowly and carefully the problem text one more time before you start:

{USER_QUERY}

In the previous message you were given a generous context -- skeletonized files.

TODO:
1. analyse thoroughly the problem statement;
2. analyse thoroughly context given (skeletonized files from the previous message);
3. among the files pick the ones (max: 5) you need to make changes to solve the problem (according to the problem statement);
4. among the files pick at least 5 more files that will give you the best context to make make changes in the chosen file (from step 3);
5. return the results in a format specified below;

Format you must obey:
[
    {
        "file_path": "/a/b/c/file.py",
        "reason": "to_change",
        "description": "contains class MyClass, body of which needs to be changed."
    },
    {
        "file_path": "/a/b/c/file1.py",
        "reason": "context",
        "description": "contains functions my_function0, my_function1 that provide useful context"
    }
    ...
]

file_path must be an absolute path.
format you return must be a valid JSON, explain nothing, don't use any quotes or backticks.

"###;

pub const SUPERCAT_EXTRACT_SYMBOLS_PROMPT: &str = r###"
Read slowly and carefully the problem text one more time before you start:

{USER_QUERY}

1. analyse thoroughly the problem statement;
2. analyse thoroughly context given (skeletonized files from the previous message);
3. from the given context select all the symbols (functino names, classes names etc) that you find relevant to the problem (either give releavant context or need to be changed);
4. return the results comma separated. Do not explain anything. Avoid backticks.

Output must be like this:
MyClass, MyFunction, MyType
"###;

pub const STRATEGY_TREE_PROMPT: &str = r###"
TODO:
1. analyse thoroughly the problem statement;
2. look thoroughly at the project tree given;
3. pick at least 10 files that will help you solving the problem (ones that give you the context and ones that shall be changed);
4. return chosen files in a json format, explain nothing.

Output must be like this:
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

Output must be like this:
[
    "variable1",
    "function11",
    "ClassName111"
]
"###;

pub const STEP1_DET_SYSTEM_PROMPT: &str = r###"
You are a genius coding assistant named "Refact". You are known for your scruplousness and well thought-out code.
Listening to the user is what makes you the best.
"###;
