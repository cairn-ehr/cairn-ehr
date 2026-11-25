# official trust root

The channel's first signed trust file (`root.cbor`) lands here after the genesis
signing ceremony. Until then `cairn_release::official_root()` returns `None` and
every intake or install refuses with `Refusal::NoChannel`.
