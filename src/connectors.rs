//! The services an Agent is connected to with a touch: those that keep a
//! door of their own open to agents and let anyone's program in on the
//! Owner's sign-in alone, with no key to make and nothing to register.
//! Read on each service's own pages on 2026-11-23.

/// A service known by name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Connector {
    /// What it is called in `mcp.json` and in the Conversation.
    pub name: &'static str,
    /// What its makers call it.
    pub label: &'static str,
    /// Where its door is.
    pub address: &'static str,
    /// What the Agent can do there, in English and in French.
    pub english: &'static str,
    pub french: &'static str,
}

const fn known(
    name: &'static str,
    label: &'static str,
    address: &'static str,
    english: &'static str,
    french: &'static str,
) -> Connector {
    Connector { name, label, address, english, french }
}

/// Every service connected with a touch, those most people use first.
/// Each was asked for a sign-in that ends on an Agent's own page, and gave
/// one, on 2026-11-23. Dropbox takes only programs it knows already, and
/// Todoist's registry did not answer that day: neither is offered.
pub const CONNECTORS: &[Connector] = &[
    known(
        "notion",
        "Notion",
        "https://mcp.notion.com/mcp",
        "Find and edit your pages and databases",
        "Retrouver et modifier tes pages et bases",
    ),
    known(
        "calendly",
        "Calendly",
        "https://mcp.calendly.com",
        "See and arrange your meetings",
        "Voir et organiser tes rendez-vous",
    ),
    known(
        "trello",
        "Trello",
        "https://mcp.trello.com/v1",
        "Manage your boards and cards",
        "Gérer tes tableaux et cartes",
    ),
    known(
        "linear",
        "Linear",
        "https://mcp.linear.app/mcp",
        "Follow and file issues and projects",
        "Suivre et créer tickets et projets",
    ),
    known(
        "atlassian",
        "Jira & Confluence",
        "https://mcp.atlassian.com/v2/mcp",
        "Work in Jira, Confluence and Bitbucket",
        "Travailler dans Jira, Confluence et Bitbucket",
    ),
    known(
        "clickup",
        "ClickUp",
        "https://mcp.clickup.com/mcp",
        "Manage your tasks and documents",
        "Gérer tes tâches et documents",
    ),
    known(
        "airtable",
        "Airtable",
        "https://mcp.airtable.com/mcp",
        "Read and fill your bases",
        "Lire et remplir tes bases",
    ),
    known(
        "stripe",
        "Stripe",
        "https://mcp.stripe.com",
        "Look at payments and invoices",
        "Consulter paiements et factures",
    ),
    known(
        "paypal",
        "PayPal",
        "https://mcp.paypal.com/http",
        "Look at invoices and orders",
        "Consulter factures et commandes",
    ),
    known(
        "sentry",
        "Sentry",
        "https://mcp.sentry.dev/mcp",
        "Look into your programs' errors",
        "Examiner les erreurs de tes programmes",
    ),
    known(
        "supabase",
        "Supabase",
        "https://mcp.supabase.com/mcp",
        "Query and look after your databases",
        "Interroger et suivre tes bases de données",
    ),
    known(
        "cloudflare",
        "Cloudflare",
        "https://mcp.cloudflare.com/mcp",
        "Look after your sites and domains",
        "Suivre tes sites et domaines",
    ),
];
