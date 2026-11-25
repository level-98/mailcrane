# ManageSieve responses

Synthetic ManageSieve (RFC 5804) server responses, CRLF line endings. They seed
the `sieve_response` fuzz target (`cargo xtask fuzz sieve_response`).

- `greeting.sieve`: a capability greeting with STARTTLS.
- `listscripts.sieve`: `LISTSCRIPTS` with an active script and escapes.
- `getscript-quota.sieve`: a literal script, a quota refusal and a `BYE`.
