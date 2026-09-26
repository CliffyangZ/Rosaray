# System One client example (curl)

Any local component that speaks `system-one/1` can drive the QKB; no image
analysis is involved. Credentials: `<project>/qkb-credentials/system-one.token`
(header `X-Rosaray-SystemOne`). The port is printed by `rosaray-service` at start.

```bash
SO=$(cat rosaray-project/qkb-credentials/system-one.token); PORT=...
RID=$(uuidgen | tr A-Z a-z)

curl -s -H "X-Rosaray-SystemOne: $SO" -H 'content-type: application/json' localhost:$PORT/qkb/v1/system-one/queries -d '{
  "protocol_version":"system-one/1","request_id":"'$RID'","task_purpose":"threshold segmentation",
  "data_conditions":{"modality":"intraoral-photo"}}'

curl -s -H "X-Rosaray-SystemOne: $SO" -H 'content-type: application/json' localhost:$PORT/qkb/v1/system-one/selections -d '{
  "protocol_version":"system-one/1","request_id":"'$RID'","decision":"selected",
  "selection":{"id":"…","version":"1.0.0","content_id":"b3:…"},"reason":"closest fit"}'
# or: "decision":"abstained" with no "selection"

curl -s -H "X-Rosaray-SystemOne: $SO" localhost:$PORT/qkb/v1/system-one/selections/$RID
```

`data_conditions` keys: `modality, color_mode, format, width_px, height_px,
pixel_spacing_mm{x,y}, mask_present, coordinate_space` (all optional; an unstated
fact is unknown and never satisfies a requirement).
