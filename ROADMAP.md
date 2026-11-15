# Roadmap

Goal: an address autocomplete covering as much of Europe as open data
allows, one index per country, served by `geocoder-api`.

## Now

- [x] France (BAN, Etalab 2.0), Luxembourg (BD-Adresses, CC0), Belgium
  (BeST Address, CC BY 4.0).
- [x] Indexes built in GitHub Actions (`build-index.yml`): measured peaks
  5.1 GiB (fr), 3.9 GiB (be), 0.2 GiB (lu) on standard runners.
- [ ] **Prebuilt indexes**: the CI publishes `v<format>/<cc>.bin` (with
  `.sha256` and `.source.json`) to object storage (OVH S3, public read);
  `geocoder-api update` and siren download them instead of building, so
  neither needs more than a few hundred MB.
- [ ] First release on crates.io (0.2.0).

## Countries

Survey of national address registers, 2026-11-15. Licences and formats
were checked on the publishers' portals unless marked *unverified*;
re-check before implementing a country.

### Tier 1: one national file, open licence, housenumbers with coordinates

| Country | Register (publisher) | Licence | Format, update | Notes |
|---|---|---|---|---|
| 🇨🇭 Switzerland + 🇱🇮 Liechtenstein | Amtliches Gebäudeadressverzeichnis (swisstopo, from the GWR) | swisstopo OGD: free, commercial use, citing swisstopo mandatory | CSV zip (CH 143 MB, LI 0.4 MB), daily, LV95 | One parser for both. Streets in DE/FR/IT/RM by commune, bilingual communes (Biel/Bienne) |
| 🇳🇱 Netherlands | BAG (Kadaster, PDOK) | CC0 | GeoPackage 7.8 GB or XML 3.6 GB, monthly, RD New | ~9–10 M addresses *(unverified)*. Largest after FR |
| 🇳🇴 Norway | Matrikkelen Adresse (Kartverket, Geonorge) | CC BY 4.0 | National CSV, weekly, ETRS89 | Sámi names in some municipalities |
| 🇱🇻 Latvia | VARIS (VZD) | CC BY 4.0 | CSV (`aw_eka.csv` 142 MB), daily | WGS84 columns included |
| 🇨🇿 Czechia | RÚIAN adresní místa (ČÚZK) | CC BY 4.0 | National CSV zip, monthly, S-JTSK | Mind the S-JTSK signs and axis order |
| 🇵🇱 Poland | PRG punkty adresowe (GUGiK) | Free for any use (statutory, no named licence) | SHP 565 MB / GML 806 MB, continuous | Attribution terms unclear |
| 🇦🇹 Austria | Adressregister Stichtagsdaten (BEV) | CC BY 4.0, credit "© Österreichisches Adressregister, Stichtagsdaten vom …" | CSV tables in a zip, per reference date | Coordinate columns *(unverified)* |
| 🇮🇸 Iceland | Staðfangaskrá (HMS) | CC BY 4.0 *(per a mirror)* | CSV, weekly, WGS84 | |
| 🇪🇪 Estonia | ADS (Maa- ja Ruumiamet) | Open (CC0 per the portal, credit requested) | CSV, monthly, L-EST97 | The FTP may block foreign IPs |
| 🇷🇸 Serbia | Adresni registar (RGZ) | Open Data License of Serbia (CC BY-like) | CSV / GPKG, weekly | ~2.4 M housenumbers. Cyrillic: Latin transliteration aliases |

Shared work: **reprojection** (most files use a national CRS: LV95, RD,
UTM 33, S-JTSK, L-EST97, MGI, …) and a **generic register reader**
(column mapping per country on top of `readers::grouped`).

### Tier 2: open, with friction

| Country | Register (publisher) | Licence | Friction |
|---|---|---|---|
| 🇪🇸 Spain | CartoCiudad (CNIG) or Catastro INSPIRE Addresses | CC BY 4.0 | Per province / municipality (GPKG or GML). Catastro excludes the Basque Country and Navarre (foral cadastres). Co-official languages (Catalan, Galician, Basque) |
| 🇫🇮 Finland | Ryhti building addresses (SYKE) | CC BY 4.0 | OGC API Features only (3.87 M features to page). Finnish and Swedish names in every record |
| 🇩🇰 Denmark | DAR (Klimadatastyrelsen, Datafordeler) | Free basic data, attribution | Free account; access moves to API key + GraphQL on 2027-01-15, DAWA shutting down: wait for it to settle |
| 🇵🇹 Portugal | Base Nacional de Moradas (INE) | CC BY (HVD) | INSPIRE GML by NUTS II region |
| 🇸🇮 Slovenia | Register naslovov (GURS) | CC BY 4.0 | Download through the JGP app, CSV for addresses *(unverified)* |
| 🇸🇰 Slovakia | Register adries (Ministry of Interior) | CC0 | XML batches; ~9 k swapped coordinates and points outside SK to clean |
| 🇱🇹 Lithuania | Adresų registras (Registrų centras) | Open with attribution *(unverified)* | get.data.gov.lt API, not confirmed |
| 🇩🇪 Germany | No open national register (HK-DE is paid); Länder files | Per Land: Berlin dl-de/zero, Saxony free, others *(unverified)* | Patchwork: free Länder first (SN, BE, NRW, TH…), OSM for the rest |
| 🇸🇪 Sweden | Belägenhetsadress (Lantmäteriet), HVD since 2025 | CC BY 4.0 plus extra terms | Application required, recipient becomes GDPR controller: check redistribution before applying |

### Tier 3: no usable open register, OpenStreetMap fallback

| Country | Why |
|---|---|
| 🇮🇹 Italy | ANNCSU is open but only ~5 % of its 27.9 M housenumbers have coordinates |
| 🇭🇺 Hungary, 🇷🇴 Romania, 🇧🇬 Bulgaria, 🇬🇷 Greece, 🇭🇷 Croatia, 🇨🇾 Cyprus | Register not released (view services or on request only) |
| 🇲🇹 Malta | Listed on the national portal, content *(unverified)* |
| 🇮🇪 Ireland | Eircode / GeoDirectory: commercial |
| 🇬🇧 United Kingdom | AddressBase: proprietary. OS Open UPRN (OGL) has coordinates but no address text |
| 🇲🇨 Monaco, 🇦🇩 Andorra, 🇸🇲 San Marino, 🇻🇦 Vatican | No official open data; small areas, OSM is fine for MC and AD |
| 🇧🇦 🇲🇪 🇲🇰 🇦🇱 🇽🇰 🇲🇩 🇺🇦 | No open download (Montenegro *unverified*, Ukraine without coordinates) |

One **generic OSM `addr:*` importer** (or OpenAddresses, which aggregates
many sources) rather than one per country. Expect good housenumber
coverage in DE, AT, CZ, PL and moderate to poor elsewhere (IE, GR, RO,
BG, UA notably weak). OSM data is ODbL: attribution, and share-alike on
the index (a derived database) if it is distributed.

## Proposed order

1. Prebuilt indexes (above): every new country then costs CI time only.
2. Tier 1, by value: CH + LI, NL, AT, CZ, PL, NO, then LV, EE, IS, RS.
3. Spain and Finland (multilingual, large), then DK after its 2027
   migration, PT, SI, SK, LT.
4. Germany: free Länder, then the OSM importer for the rest of DE and for
   tier 3.
5. Sweden, depending on the terms.

## Cross-cutting

- **A profile per language family** rather than per country: the French
  processing serves FR/LU/BE/CH-romand; German, Dutch, Italian, Spanish,
  Nordic and Slavic profiles would cover most of the rest (housenumber
  position, street types and synonyms, compounds).
- **Multilingual aliases**: every official name of a street indexed for
  the same point, the label in the language of the query (done for BE).
- **Transliteration** for Cyrillic and Greek (RS, BG, GR, UA).
- **Cross-border search**: one query over several indexes (e.g. a
  Benelux or DACH autocomplete), merged by score.
- **Attribution** per country, returned with every response (done).
