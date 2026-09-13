//! # archie-search
//!
//! Web search, entered through the capability model rather than around it.
//!
//! # The path
//!
//! ```text
//! Cerebra → SearchGate → TavilyClient → api.tavily.com
//! ```
//!
//! A query is a disclosure. Whatever words Archie sends to a search provider leave the
//! machine, and a query assembled from someone's private notes discloses those notes as
//! surely as a prompt would. So a search is not a function call that happens to use the
//! network; it is the same decision a tool call is, and it passes the same kind of gate:
//!
//! * [`manifest`] — what the operator declared about a provider, validated before it can
//!   be registered. A cloud provider whose retention nobody has verified can be cleared
//!   for public queries and nothing else; a manifest claiming more is invalid rather than
//!   permissive.
//! * [`gate`] — admission. [`AuthorizedSearch`] has no public constructor, so the client
//!   cannot be handed a query the gate did not see. A model's suggested query is a
//!   [`SearchProposal`], which nothing sends.
//! * [`tavily`] — the network boundary. HTTPS only, no redirects, a response budget
//!   enforced before allocation, and the API key brokered from the vault per call.
//! * [`results`] — what comes back, treated as third-party data: bounded, stripped of
//!   control and bidirectional-override characters, and given a stable `web:` reference
//!   so a later stage can cite a result without quoting it.
//!
//! # Why a native client and not Tavily's MCP server
//!
//! Tavily offers a remote MCP server, and `archie-mcp` can reach one. Two things decided
//! against it. An MCP tool result is a block of prose, and Archie's reasoning path cites
//! evidence by reference and refuses citations it cannot check; a typed result with its
//! URL is citable, a paragraph is not. And the privacy rule for a query includes the
//! provider's retention posture, which a search manifest here carries and enforces.
//!
//! # What is never requested
//!
//! Tavily's generated `answer` (model output that did not pass Archie's own gates), raw
//! page content (large, and a separate budget decision), images, the operator's country,
//! and `auto_parameters` (which lets the provider choose the expensive depth). The request
//! says `false` for each explicitly, so a change in the provider's defaults cannot turn
//! them on.
//!
//! # Not yet here
//!
//! Tavily's `extract`, `crawl` and `map` operations, and credit accounting. Retention is
//! `unknown` until an operator verifies the account's setting; nothing in this crate can
//! establish it.

#![forbid(unsafe_code)]

pub mod gate;
pub mod manifest;
pub mod results;
pub mod tavily;

pub use gate::{
    AuthorizedSearch, SearchAdmission, SearchAuthority, SearchDepth, SearchGate, SearchOptions,
    SearchProposal, SearchReceipt, SearchRefusal, SearchTopic, TimeRange, MAX_DOMAIN_FILTERS,
    MAX_QUERY_CHARS,
};
pub use manifest::{
    ManifestError, SearchProviderKind, SearchProviderManifest, SearchRegistry, PROVIDER_MAX_RESULTS,
};
pub use results::{SearchResults, WebResult};
pub use tavily::{SearchError, TavilyClient};
