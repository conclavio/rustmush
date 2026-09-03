-- KEYS: intent, receipt, idempotency binding, pending set, retained count,
-- retained bytes, object allocator, then every ObjectSet target.
-- ARGV: intent doc, binding doc, token, candidate bytes, max count, max bytes,
-- proposed allocator (empty when absent), object count, then ObjectSet ids.

local function key_type(key)
  local result = redis.call('TYPE', key)
  if type(result) == 'table' then
    return result['ok']
  end
  return result
end

local pending_type = key_type(KEYS[4])
if pending_type ~= 'none' and pending_type ~= 'set' then
  return 'CORRUPT_PENDING'
end

local existing_intent = redis.call('GET', KEYS[1])
if existing_intent then
  if existing_intent ~= ARGV[1] then
    return 'TOKEN_CONFLICT'
  end
  local existing_binding = redis.call('GET', KEYS[3])
  if existing_binding ~= ARGV[2] then
    return 'CORRUPT_BINDING'
  end
  return 'RESUMED'
end

if redis.call('EXISTS', KEYS[2]) ~= 0 then
  return 'CORRUPT_RECEIPT'
end

local existing_binding = redis.call('GET', KEYS[3])
if existing_binding then
  if existing_binding == ARGV[2] then
    return 'CORRUPT_ORPHAN_BINDING'
  end
  return 'IDEMPOTENCY_CONFLICT'
end

local count_doc = redis.call('GET', KEYS[5])
local bytes_doc = redis.call('GET', KEYS[6])
if (count_doc and not bytes_doc) or (bytes_doc and not count_doc) then
  return 'CORRUPT_COUNTER'
end
local retained_count = tonumber(count_doc or '0')
local retained_bytes = tonumber(bytes_doc or '0')
local candidate_bytes = tonumber(ARGV[4])
local max_count = tonumber(ARGV[5])
local max_bytes = tonumber(ARGV[6])
if not retained_count or not retained_bytes or not candidate_bytes or
   not max_count or not max_bytes or retained_count < 0 or retained_bytes < 0 then
  return 'CORRUPT_COUNTER'
end
local attempted_count = retained_count + 1
local attempted_bytes = retained_bytes + candidate_bytes
if attempted_count > max_count or attempted_bytes > max_bytes then
  return 'CAPACITY:' .. tostring(attempted_count) .. ':' .. tostring(attempted_bytes)
end

local allocator_doc = redis.call('GET', KEYS[7])
local current_next_id = tonumber(allocator_doc or '0')
if not current_next_id or current_next_id < 0 then
  return 'CORRUPT_ALLOCATOR'
end
local proposed_next_id = nil
if ARGV[7] ~= '' then
  proposed_next_id = tonumber(ARGV[7])
  if not proposed_next_id or proposed_next_id < current_next_id then
    return 'INVALID_ALLOCATOR'
  end
end

local object_count = tonumber(ARGV[8])
if not object_count or object_count < 0 or object_count ~= (#KEYS - 7) then
  return 'CORRUPT_ARGUMENTS'
end
for index = 1, object_count do
  local object_id = tonumber(ARGV[8 + index])
  if not object_id or object_id < 0 then
    return 'CORRUPT_ARGUMENTS'
  end
  local exists = redis.call('EXISTS', KEYS[7 + index]) ~= 0
  if exists then
    if not allocator_doc or object_id >= current_next_id then
      return 'CORRUPT_OBJECT_ALLOCATOR'
    end
  else
    if not proposed_next_id then
      return 'MISSING_ALLOCATOR'
    end
    if object_id < current_next_id then
      return 'ID_REUSE'
    end
    if object_id >= proposed_next_id then
      return 'INVALID_ALLOCATOR'
    end
  end
end

redis.call('SET', KEYS[1], ARGV[1])
redis.call('SET', KEYS[3], ARGV[2])
redis.call('SADD', KEYS[4], ARGV[3])
redis.call('SET', KEYS[5], tostring(attempted_count))
redis.call('SET', KEYS[6], tostring(attempted_bytes))
return 'NEW'
