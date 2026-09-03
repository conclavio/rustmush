-- Repair non-authoritative capacity indexes from an exhaustive validated
-- boot scan. KEYS: retained count, retained bytes. ARGV: exact count, bytes.

local function key_type(key)
  local result = redis.call('TYPE', key)
  if type(result) == 'table' then
    return result['ok']
  end
  return result
end

local expected_count = tonumber(ARGV[1])
local expected_bytes = tonumber(ARGV[2])
if not expected_count or not expected_bytes or
   expected_count < 0 or expected_bytes < 0 then
  return 'INVALID_USAGE'
end

local count_type = key_type(KEYS[1])
local bytes_type = key_type(KEYS[2])
if count_type == 'none' and bytes_type == 'none' and
   expected_count == 0 and expected_bytes == 0 then
  return 'UNCHANGED'
end
if count_type == 'string' and bytes_type == 'string' and
   redis.call('GET', KEYS[1]) == ARGV[1] and
   redis.call('GET', KEYS[2]) == ARGV[2] then
  return 'UNCHANGED'
end

-- SET replaces a wrong-typed derivative key. The immutable intent documents,
-- bindings, receipts, and pending index are not touched.
redis.call('SET', KEYS[1], ARGV[1])
redis.call('SET', KEYS[2], ARGV[2])
return 'REPAIRED'
