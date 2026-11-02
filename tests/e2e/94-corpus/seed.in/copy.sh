# The corpus lives once, in fixtures/; the harness says where. `hist/N/hist-rfc.md` is the RFC's
# Version history under one file name.
cp -R "$ANNOTARY_E2E_FIXTURES/corpus" "$ANNOTARY_E2E_FIXTURES/assets" .
for n in 1 2 3 4; do
  mkdir -p "hist/$n"
  cp "corpus/rfc-tile-cache/v$n.md" "hist/$n/hist-rfc.md"
done
