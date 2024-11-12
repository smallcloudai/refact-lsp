pub mod integr_github;
pub mod integr_gitlab;
pub mod integr_jira;
pub mod integr_pdb;
pub mod integr_chrome;
pub mod sessions;
pub mod process_io_utils;
pub mod integr_postgres;

pub const INTEGRATIONS_DEFAULT_YAML: &str = r#"# This file is used to configure integrations in Refact Agent.
# If there is a syntax error in this file, no integrations will work.
#
# Here you can set up which commands require confirmation or must be denied. If both apply, the command is denied.
# Rules use glob patterns for wildcard matching (https://en.wikipedia.org/wiki/Glob_(programming))
#

commands_need_confirmation:
  - "gh * delete*"
  - "glab * delete*"
  - "jira * delete*"
  - "psql*[!SELECT]*"
commands_deny:
  - "gh auth token*"
  - "glab auth token*"


# GitHub integration
#github:
#   GH_TOKEN: "GH_xxx"                      # To get a token, check out https://docs.github.com/en/authentication/keeping-your-account-and-data-secure/managing-your-personal-access-tokens
#   gh_binary_path: "/opt/homebrew/bin/gh"  # Uncomment to set a custom path for the gh binary, defaults to "gh"


# GitLab integration: install on mac using "brew install glab"
#gitlab:
#   GITLAB_TOKEN: "glpat-xxx"                   # To get a token, check out https://docs.gitlab.com/ee/user/profile/personal_access_tokens
#   glab_binary_path: "/opt/homebrew/bin/glab"  # Uncomment to set a custom path for the glab binary, defaults to "glab"


# Jira integration: install on mac using "brew tap ankitpokhrel/jira-cli && brew install jira-cli"
#jira:
#   JIRA_TOKEN: "ATAxxx"                      # To get a token, check out https://id.atlassian.com/manage-profile/security/api-tokens
#   jira_binary_path: "/opt/homebrew/bin/jira"  # Uncomment to set a custom path for the jira binary, defaults to "jira"


# Python debugger
#pdb:
#  python_path: "/opt/homebrew/bin/python3"  # Uncomment to set a custom python path, defaults to "python3"


# Chrome web browser
chrome:
  # This can be path to your chrome binary. You can install with "npx @puppeteer/browsers install chrome@stable", read
  # more here https://developer.chrome.com/blog/chrome-for-testing/?utm_source=Fibery&utm_medium=iframely
  #chrome_path: "/Users/me/my_path/chrome/mac_arm-130.0.6723.69/chrome-mac-arm64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing"
  # Or you can give it ws:// path, read more here https://developer.chrome.com/docs/devtools/remote-debugging/local-server/
  # In that case start chrome with --remote-debugging-port
  chrome_path: "ws://127.0.0.1:6006/"
  window_size: [1024, 768]
  idle_browser_timeout: 600


# Postgres database
#postgres:
#  psql_binary_path: "/path/to/psql"  # Uncomment to set a custom path for the psql binary, defaults to "psql"
#  connection_string: "postgresql://username:password@localhost/dbname"  # To get a connection string, check out https://www.postgresql.org/docs/current/libpq-connect.html#LIBPQ-CONNSTRING


# Command line: things you can call and immediately get an answer
#cmdline:
#  run_make:
#    command: "make"
#    command_workdir: "%project_path%"
#    timeout: 600
#    description: "Run `make` inside a C/C++ project, or a similar project with a Makefile."
#    parameters:    # this is what the model needs to produce, you can use %parameter% in command and workdir
#      - name: "project_path"
#        description: "absolute path to the project"
#    output_filter:                   # output filter is optional, can help if the output is very long to reduce it, preserving valuable information
#      limit_lines: 50
#      limit_chars: 10000
#      valuable_top_or_bottom: "top"  # the useful infomation more likely to be at the top or bottom? (default "top")
#      grep: "(?i)error|warning"      # in contrast to regular grep this doesn't remove other lines from output, just prefers matching when approaching limit_lines or limit_chars (default "(?i)error")
#      grep_context_lines: 5          # leave that many lines around a grep match (default 5)
#      remove_from_output: "process didn't exit"    # some lines and very long and unwanted, this is also a regular expression (default "")

"#;
