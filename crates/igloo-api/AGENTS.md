# igloo-api

The public API contract: REST request and response types, `Problem`, utoipa schemas, and
conversions to and from `igloo-core` types. No HTTP code, no logic.

- Inbound conversions are `TryFrom` collecting every field error; outbound are `From`.
- Public enums and structs are `#[non_exhaustive]`.
- Any change here changes `schemas/openapi.json`. Run
  `UPDATE_SCHEMAS=1 cargo nextest run -p igloo-control` and treat the diff as a contract change.
