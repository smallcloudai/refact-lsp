pub const COMPILED_IN_INITIAL_PRIVACY_YAML: &str = r#"#
# This config file determines if Refact is allowed to read, index or send a file to remote servers.
#
# If you have a syntax error in this file, the refact-lsp will revert to the default "block everything".
#
# Uses glob patterns: https://en.wikipedia.org/wiki/Glob_(programming)
#
# The most restrictive rule applies if a file matches multiple patterns.

privacy_rules:
    blocked:
        - "*/secret_project1/*"           # Don't forget leading */ if you are matching directory names
        - "*/secret_project2/*.txt"
        - "*.pem"

    # Restrict files to self-hosted and Refact servers; code completion works, but they are not sent to third-party models like GPT.
    only_send_to_servers_I_control:       
        - "secret_passwords.txt"


# See unit tests in privacy.rs for more examples.
"#;
