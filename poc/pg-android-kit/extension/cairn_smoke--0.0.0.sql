-- Hand-written install SQL (architecture-independent). cargo-pgrx normally
-- generates this by dlopen()ing the built .so, which is impossible for an ARM
-- .so on an x86/arm-mac host (spike 0003 §5), so we author it directly. Each
-- function binds to its pgrx V1 *_wrapper symbol in the module.
CREATE FUNCTION cairn_smoke_answer() RETURNS integer
  AS 'MODULE_PATHNAME', 'cairn_smoke_answer_wrapper' LANGUAGE c STRICT;
CREATE FUNCTION cairn_smoke_add(integer, integer) RETURNS integer
  AS 'MODULE_PATHNAME', 'cairn_smoke_add_wrapper' LANGUAGE c STRICT;
CREATE FUNCTION cairn_smoke_echo(text) RETURNS text
  AS 'MODULE_PATHNAME', 'cairn_smoke_echo_wrapper' LANGUAGE c STRICT;
CREATE FUNCTION cairn_smoke_spi() RETURNS bigint
  AS 'MODULE_PATHNAME', 'cairn_smoke_spi_wrapper' LANGUAGE c;
