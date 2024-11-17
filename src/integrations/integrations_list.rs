use crate::integrations::integr_abstract::IntegrationTrait;
// use crate::integrations::integr_chrome::ToolChrome;
// use crate::integrations::integr_github::ToolGithub;
// use crate::integrations::integr_gitlab::ToolGitlab;
// use crate::integrations::integr_pdb::ToolPdb;
use crate::integrations::integr_postgres::ToolPostgres;


pub fn integration_from_name(n: &String) -> Box<dyn IntegrationTrait + Send + Sync>
{
    match n.as_str() {
        // "github" => Box::new(ToolGithub { ..Default::default() }) as Box<dyn IntegrationTrait + Send + Sync>,
        // "gitlab" => Box::new(ToolGitlab { ..Default::default() }) as Box<dyn IntegrationTrait + Send + Sync>,
        // "pdb" => Box::new(ToolPdb { ..Default::default() }) as Box<dyn IntegrationTrait + Send + Sync>,
        "postgres" => Box::new(ToolPostgres { ..Default::default() }) as Box<dyn IntegrationTrait + Send + Sync>,
        // "chrome" => Box::new(ToolChrome { ..Default::default() }) as Box<dyn IntegrationTrait + Send + Sync>,
        _ => panic!("Unknown integration name: {}", n),
    }
}

pub fn integration_list() -> Vec<&'static str> {
    vec![
        // "github",
        // "gitlab",
        // "pdb",
        "postgres",
        // "chrome"
    ]
}
