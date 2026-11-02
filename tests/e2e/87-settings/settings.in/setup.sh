#!/bin/sh
# Member @kim with profile "kim" on the harness server (the golden cannot spell its URL), plus
# three of Kim's Devices made the way the clients make them: Web (`?device=web`, as a browser
# sign-in does) and an iPhone and an iPad that send `X-Annotary-Device`, each with a Token of
# its own used from it twice (the second request stamps the Token's Device, so `devices forget`
# has a Token to revoke). The CLI itself never names a Device.
set -e
url=$(sed -n 's/^server = "\(http:\/\/127\.0\.0\.1:[0-9]*\)"$/\1/p' "$HOME/.annotary/config.toml" | grep -v ':1$' | head -n 1)
tok() { sed -n 's/^.*ANNOTARY_TOKEN=\(ant_[A-Za-z0-9_-]*\).*$/\1/p' | head -n 1; }
kim=$(annotary -p rowan member add kim --name "Kim" 2>&1 | tok)
annotary -p rowan profile add kim --server "$url" --token "$kim"
phone=$(annotary -p rowan token mint --member kim --name iphone 2>&1 | tok)
pad=$(annotary -p rowan token mint --member kim --name ipad 2>&1 | tok)
get() {
  curl -sf -o /dev/null -H "Authorization: Bearer $1" -H "X-Annotary-Device: $2" \
    -H "X-Annotary-Device-Info: $3" "$url/api/v1/me/settings"
}
curl -sf -o /dev/null -H "Authorization: Bearer $kim" "$url/api/v1/me/settings?device=web"
for _ in 1 2; do
  get "$phone" 01K9PH0NE00000000000000001 "kind=iphone; name=iPhone 17"
  get "$pad" 01K9PAD0000000000000000002 "kind=ipad; name=iPad Pro"
done
echo "Kim has Web, an iPhone and an iPad"
