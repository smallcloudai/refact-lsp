pub mod integr_github;

pub const INTEGRATIONS_DEFAULT_YAML: &str = r#"# This file is used to configure integrations.
#
# If there is a syntax error in this file, integrations will not be loaded.

# This part is used to configure GitHub integration.
#
# By default, commands will ask for confirmation. You can set rules to skip confirmation or deny 
# specific commands using glob patterns (https://en.wikipedia.org/wiki/Glob_(programming))
#
# `deny` overrides `skip_confirmation` if both match
github:
  GH_TOKEN: # Your GITHUB_TOKEN (see https://docs.github.com/en/authentication/keeping-your-account-and-data-secure/managing-your-personal-access-tokens)
  # gh_binary_path: # Uncomment to set a custom path for the gh binary, defaults to gh in PATH
  skip_confirmation: 
    - "{browse,search,status}*"
    - "{repo,gist,issue,org,pr,project,release,cache,run,workflow,label,ruleset} {list,view}*"
  deny: 
    - "auth token*"
"#;
