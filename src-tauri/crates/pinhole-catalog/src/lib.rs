//! CivitAI catalog (SPEC §5.4): API client over `pinhole_net::HttpClient`,
//! plain-language filters from `config/catalog-filters.yaml`, safe file
//! selection (SafeTensor/GGUF only, pickle + virus scans must be Success), and
//! model-card view models.
//!
//! OWNER: catalog agent. Suggested modules: api (types + client), filters,
//! select, cards.
