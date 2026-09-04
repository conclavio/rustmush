-- Atomically restore one exact full intent from an independently verified
-- quarantine record. No replay target is touched here.
--
-- KEYS: intent, finalized receipt, idempotency binding, pending set,
-- retained count, retained bytes, retained generation, quarantine receipt.
-- ARGV: exact intent doc, exact binding doc, exact quarantine receipt, token,
-- candidate bytes, max count, max bytes.

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

local pending_type = key_type(KEYS[4])
if pending_type ~= 'none' and pending_type ~= 'set' then
  return 'CORRUPT_PENDING'
end

local intent = redis.call('GET', KEYS[1])
if intent then
  if intent ~= ARGV[1] then
    return 'TOKEN_CONFLICT'
  end
  if redis.call('GET', KEYS[3]) ~= ARGV[2] then
    return 'CORRUPT_BINDING'
  end
  if redis.call('EXISTS', KEYS[8]) ~= 0 then
    return 'CORRUPT_QUARANTINE'
  end
  if redis.call('EXISTS', KEYS[2]) == 0 then
    redis.call('SADD', KEYS[4], ARGV[4])
  else
    redis.call('SREM', KEYS[4], ARGV[4])
  end
  return 'RESUMED'
end

if redis.call('EXISTS', KEYS[2]) ~= 0 then
  return 'ALREADY_FINALIZED'
end
if redis.call('GET', KEYS[3]) ~= ARGV[2] then
  return 'CORRUPT_BINDING'
end
if redis.call('GET', KEYS[8]) ~= ARGV[3] then
  if redis.call('EXISTS', KEYS[8]) == 0 then
    return 'MISSING_INTENT'
  end
  return 'TOKEN_CONFLICT'
end

local pending_count = redis.call('SCARD', KEYS[4])
if pending_count > 1 then
  return 'CORRUPT_PENDING'
end
if pending_count ~= 0 then
  return 'INTENT_IN_FLIGHT:' .. tostring(pending_count)
end

local count_doc = redis.call('GET', KEYS[5])
local bytes_doc = redis.call('GET', KEYS[6])
if (count_doc and not bytes_doc) or (bytes_doc and not count_doc) then
  return 'CORRUPT_COUNTER'
end
local retained_count = tonumber(count_doc or '0')
local retained_bytes = tonumber(bytes_doc or '0')
local candidate_bytes = tonumber(ARGV[5])
local max_count = tonumber(ARGV[6])
local max_bytes = tonumber(ARGV[7])
if not retained_count or not retained_bytes or not candidate_bytes or
   not max_count or not max_bytes or retained_count < 0 or retained_bytes < 0 then
  return 'CORRUPT_COUNTER'
end
local attempted_count = retained_count + 1
local attempted_bytes = retained_bytes + candidate_bytes
if attempted_count > max_count or attempted_bytes > max_bytes then
  return 'CAPACITY:' .. tostring(attempted_count) .. ':' .. tostring(attempted_bytes)
end

local generation_type = key_type(KEYS[7])
if generation_type ~= 'none' and generation_type ~= 'string' then
  return 'CORRUPT_GENERATION'
end
local generation = redis.call('GET', KEYS[7]) or '0'
if not canonical_decimal(generation) or
   (string.len(generation) == 19 and generation > '9223372036854775806') or
   string.len(generation) > 19 then
  return 'CORRUPT_GENERATION'
end

redis.call('SET', KEYS[1], ARGV[1])
redis.call('DEL', KEYS[8])
redis.call('SADD', KEYS[4], ARGV[4])
redis.call('SET', KEYS[5], tostring(attempted_count))
redis.call('SET', KEYS[6], tostring(attempted_bytes))
redis.call('INCR', KEYS[7])
return 'RESTORED'
