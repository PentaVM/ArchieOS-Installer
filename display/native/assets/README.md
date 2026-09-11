# Native system GUI assets

Both textures are NASA public-domain imagery, embedded into the native `archie-display`
executable so the globe needs no network and no licence at run time. Neither is live
imagery: live data is rendered only as a separate typed layer with provider identity,
timestamps and health.

Every entry records the upstream URL and the SHA-256 **of the file as shipped**, so the
provenance can be checked rather than believed:

```
sha256sum display/native/assets/*.jpg display/native/assets/*.png
```

## `earth-bmng-5400.jpg` — the globe at full detail

- Upstream: `https://eoimages.gsfc.nasa.gov/images/imagerecords/73000/73909/world.topo.bathy.200412.3x5400x2700.jpg`
- SHA-256: `a9f0088972dee0254610af851c4d6838ca3f2cf79176987e0a5713e2c15ec042`
- Size: 2,566,770 bytes. 5400 × 2700.
- Blue Marble Next Generation, December 2004, topography and bathymetry.
- Credit: NASA Earth Observatory; Reto Stöckli, NASA Goddard Space Flight Center.

Shipped as the bytes NASA serves, not a re-encode. A recompressed copy renders the same
and its hash matches nothing anybody can fetch, which makes the record above decorative.

## `earth-blue-marble.png` — the globe on a conserving tier

- Upstream: `https://eoimages.gsfc.nasa.gov/images/imagerecords/57000/57730/land_ocean_ice_2048.png`
- SHA-256: `b5e0139834c638d10c2c747f4bac63df5f9387680c00d87e5a2a9ed9a3dfea71`
- Size: 1,912,469 bytes. 2048 × 1024.
- Credit: NASA Goddard Space Flight Center; Reto Stöckli, Robert Simmon,
  MODIS Land Group, MODIS Science Data Support Team, MODIS Atmosphere Group,
  MODIS Ocean Group; additional topography and polar data from USGS and NOAA.

Kept because the render tier chooses between them. A machine that has told the observatory
it needs its GPU back should not be uploading a 5400-pixel texture, and dropping to the
smaller one is a real reduction rather than a label.

## Why not Google Earth imagery

It cannot be shipped. Google's photorealistic tiles require a billed API key and their
terms forbid caching or redistributing the imagery, so it cannot live inside an ISO at
any price. What makes that view beautiful from orbit is not the imagery anyway — it is
the layering: colour, terrain relief, ocean specular, night lights and atmospheric
scattering. All five have public-domain sources. The atmosphere is in
`display/native/shaders/`; the rest are open.
