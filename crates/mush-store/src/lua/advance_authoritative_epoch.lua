-- Advance one scope's minimum accepted epoch only if the retained-intent
-- generation still matches the value observed before the caller's exhaustive
-- scan. This closes the scan/reservation race without trusting an index.
--
-- KEYS: minimum epoch, retained generation.
-- ARGV: observed generation, requested minimum epoch.

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

local minimum_type = key_type(KEYS[1])
local generation_type = key_type(KEYS[2])
if (minimum_type ~= 'none' and minimum_type ~= 'string') or
   (generation_type ~= 'none' and generation_type ~= 'string') then
  return 'CORRUPT_WATERMARK_STATE'
end

local current = redis.call('GET', KEYS[1]) or '0'
local generation = redis.call('GET', KEYS[2]) or '0'
if not canonical_decimal(current) or not canonical_decimal(generation) or
   not canonical_decimal(ARGV[1]) or not canonical_decimal(ARGV[2]) then
  return 'CORRUPT_WATERMARK_STATE'
end
if decimal_less(ARGV[2], current) then
  return 'REGRESSION:' .. current
end
if generation ~= ARGV[1] then
  return 'RETRY'
end
if current == ARGV[2] then
  return 'RESUMED:' .. current
end

redis.call('SET', KEYS[1], ARGV[2])
return 'ADVANCED:' .. current
