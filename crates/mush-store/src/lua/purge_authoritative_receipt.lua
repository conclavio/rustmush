-- Remove one exact compact receipt/idempotency tombstone pair after its
-- scope watermark has committed. Exact byte comparisons prevent deletion of
-- a concurrently or externally replaced record.
--
-- KEYS: receipt, idempotency binding, full intent, minimum epoch.
-- ARGV: exact receipt doc, exact binding doc, required minimum epoch.

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

local receipt = redis.call('GET', KEYS[1])
local binding = redis.call('GET', KEYS[2])
if not receipt and not binding then
  return 'RESUMED'
end
if receipt ~= ARGV[1] or binding ~= ARGV[2] then
  return 'TOMBSTONE_CONFLICT'
end

redis.call('DEL', KEYS[1], KEYS[2])
return 'PURGED'
