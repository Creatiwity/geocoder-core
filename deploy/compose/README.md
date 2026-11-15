# geocoder-api with Docker Compose

For a single host (VPS): the API behind Caddy (automatic HTTPS) and an
`update` service run by the host's cron.

## Sizing

| | Memory | Disk |
|---|---|---|
| Serving fr + lu + be | ~60 MB private + the page cache of the indexes it touches (reclaimable) | 2.9 GB of indexes |
| Building fr | ~5 GB peak, ~6 min | + ~2.7 GB rebuild + 1.2 GB download |
| Building be / lu | ~4.5 GB peak, ~1 min / a few seconds | small |

Measured on 3,000 mixed queries: 189 MB resident (of which ~120 MB index
pages), 3 ms per query. A host with 8 GB builds and serves; with 4 GB, add
swap for the builds (they are slower, serving is unaffected).

## Install

```bash
cp .env.example .env && $EDITOR .env    # GEOCODER_DOMAIN at least
docker compose up -d
docker compose run --rm update          # first build: ~10 min for fr+lu+be
curl https://$GEOCODER_DOMAIN/countries
```

Until the first build, searches answer 503. The API picks every new index
up within a minute, without a restart.

## Updates

`update` does nothing when the sources did not change (BAN: daily, LU and
BE: weekly). Host crontab:

```cron
0 3 * * * cd /opt/geocoder && docker compose run --rm update >> /var/log/geocoder-update.log 2>&1
```

Force a rebuild: `docker compose run --rm update update --force`; one
country: `docker compose run --rm update update --country be`.

## Swap (4 GB hosts)

```bash
fallocate -l 4G /swapfile && chmod 600 /swapfile && mkswap /swapfile && swapon /swapfile
echo '/swapfile none swap sw 0 0' >> /etc/fstab
```
