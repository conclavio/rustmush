-- Repair non-authoritative capacity indexes from an exhaustive validated
-- boot scan. KEYS: retained count, retained bytes, retained generation.
-- ARGV: exact count, bytes, pre-scan generation.

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

local expected_count = tonumber(ARGV[1])
local expected_bytes = tonumber(ARGV[2])
local expected_generation = ARGV[3]
if not expected_count or not expected_bytes or
   expected_count < 0 or expected_bytes < 0 or
   not canonical_decimal(expected_generation) then
  return 'INVALID_USAGE'
end

local generation_type = key_type(KEYS[3])
if generation_type ~= 'none' and generation_type ~= 'string' then
  return 'CORRUPT_GENERATION'
end
local generation = redis.call('GET', KEYS[3]) or '0'
if not canonical_decimal(generation) then
  return 'CORRUPT_GENERATION'
end
if generation ~= expected_generation then
  return 'GENERATION_CHANGED'
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
