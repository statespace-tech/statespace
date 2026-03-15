---
tools:
  - [curl, -s, http://api.open-notify.org/iss-now.json, ;]
  - [curl, -s, { regex: "^https://nominatim\\.openstreetmap\\.org/reverse\\?format=json&lat=.*&lon=.*$" }, ;]
  - [curl, -s, { regex: "^https://en\\.wikipedia\\.org/api/rest_v1/page/summary/.+$" }, ;]
---

# Where is the ISS?

Track the International Space Station, predict where it's heading, and learn about that place.

## Steps

**1. Locate** — Call the ISS position API to get current latitude and longitude.

**2. Geocode** — Pass the coordinates to OpenStreetMap to identify what's below. If it's over ocean, call the ISS API again — compare the two positions to determine heading and speed, then extrapolate to find the next landmass in its path. Geocode that predicted position instead.

**3. Learn** — Look up the identified place on Wikipedia.

## Questions you can ask

- Where is the ISS right now and what's below it?
- Where is the ISS heading next?
