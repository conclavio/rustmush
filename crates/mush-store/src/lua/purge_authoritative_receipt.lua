-- Remove one exact compact receipt/idempotency tombstone pair after its
-- scope watermark has committed. Exact byte comparisons prevent deletion of
-- a concurrently or externally replaced record.
--
-- KEYS: receipt, idempotency binding, full intent, minimum epoch, exact
-- parent outbox manifest, outbox pending index, then every terminal item.
-- ARGV: exact receipt doc, exact binding doc, required minimum epoch, exact
-- parent manifest, item count, then (item id, exact item doc) pairs.

local function key_type(key)
  local result = redis.call('TYPE', key)
  if type(result) == 'table' then
    return result['ok']
  end
  return result
end

local function canonical_decimal(value)
  return value and string.match(value, '^%d+$') and
    (value == '0' or string.sub(value, 1, 1) ~= '0')
end

local function decimal_less(left, right)
  if string.len(left) ~= string.len(right) then
    return string.len(left) < string.len(right)
  end
  return left < right
end

if key_type(KEYS[4]) ~= 'string' then
  return 'MISSING_WATERMARK'
end
local current = redis.call('GET', KEYS[4])
if not canonical_decimal(current) or not canonical_decimal(ARGV[3]) or
   decimal_less(current, ARGV[3]) then
  return 'MISSING_WATERMARK'
end
if redis.call('EXISTS', KEYS[3]) ~= 0 then
  return 'RETAINED_INTENT'
end

local pending_type = key_type(KEYS[6])
if pending_type ~= 'none' and pending_type ~= 'set' then
  return 'CORRUPT_OUTBOX_PENDING'
end

local item_count = tonumber(ARGV[5])
if not item_count or item_count < 0 or item_count ~= (#KEYS - 6) or
   #ARGV ~= (5 + item_count * 2) then
  return 'CORRUPT_ARGUMENTS'
end
local parent_manifest = redis.call('GET', KEYS[5])
if parent_manifest and parent_manifest ~= ARGV[4] then
  return 'TOMBSTONE_CONFLICT'
end
if not parent_manifest and ARGV[4] ~= '[]' then
  return 'TOMBSTONE_CONFLICT'
end
for index = 1, item_count do
  local item_id = ARGV[4 + index * 2]
  local item_doc = ARGV[5 + index * 2]
  if redis.call('GET', KEYS[6 + index]) ~= item_doc then
    return 'TOMBSTONE_CONFLICT'
  end
  if redis.call('SISMEMBER', KEYS[6], item_id) ~= 0 then
    return 'OUTBOX_PENDING'
  end
end

local receipt = redis.call('GET', KEYS[1])
local binding = redis.call('GET', KEYS[2])
if not receipt and not binding then
  redis.call('DEL', KEYS[5])
  return 'RESUMED'
end
if receipt ~= ARGV[1] or binding ~= ARGV[2] then
  return 'TOMBSTONE_CONFLICT'
end

for index = 1, item_count do
  redis.call('DEL', KEYS[6 + index])
end
redis.call('DEL', KEYS[1], KEYS[2], KEYS[5])
return 'PURGED'
