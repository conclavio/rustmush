-- KEYS: intent, receipt, idempotency binding, pending set, outbox pending
-- set, exact parent outbox manifest, then one key per normalized replay write, then one key per durable
-- outbox item.
-- ARGV: exact intent doc, exact receipt doc, exact binding doc, token,
-- replay-write count, outbox count, max pending outbox, exact parent manifest, then
-- (kind, exact post-image) replay pairs and (item id, exact record) pairs.
--
-- Returns 'FINALIZED', or 'FINALIZED_RENORMALISED:<replay index>' when the
-- write landed and the backend spelled one JSON post-image back differently
-- (see the readback loop), or one of the refusal codes above.

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
local outbox_pending_type = key_type(KEYS[5])
if outbox_pending_type ~= 'none' and outbox_pending_type ~= 'set' then
  return 'CORRUPT_OUTBOX_PENDING'
end
local parent_manifest_type = key_type(KEYS[6])
if parent_manifest_type ~= 'none' and parent_manifest_type ~= 'string' then
  return 'OUTBOX_CONFLICT'
end

local intent = redis.call('GET', KEYS[1])
if not intent then
  return 'MISSING_INTENT'
end
if intent ~= ARGV[1] then
  return 'TOKEN_CONFLICT'
end
if redis.call('GET', KEYS[3]) ~= ARGV[3] then
  return 'CORRUPT_BINDING'
end

local receipt = redis.call('GET', KEYS[2])
if receipt then
  if receipt == ARGV[2] then
    local parent_manifest = redis.call('GET', KEYS[6])
    if parent_manifest and parent_manifest ~= ARGV[8] then
      return 'OUTBOX_CONFLICT'
    end
    if not parent_manifest then
      redis.call('SET', KEYS[6], ARGV[8])
    end
    redis.call('SREM', KEYS[4], ARGV[4])
    return 'FINALIZED'
  end
  return 'TOKEN_CONFLICT'
end

local write_count = tonumber(ARGV[5])
local outbox_count = tonumber(ARGV[6])
local max_pending_outbox = tonumber(ARGV[7])
if not write_count or write_count < 0 or not outbox_count or
   outbox_count < 0 or not max_pending_outbox or max_pending_outbox < 0 or
   not ARGV[8] or (write_count + outbox_count) ~= (#KEYS - 6) then
  return 'CORRUPT_ARGUMENTS'
end

local existing_parent_manifest = redis.call('GET', KEYS[6])
if existing_parent_manifest and existing_parent_manifest ~= ARGV[8] then
  return 'OUTBOX_CONFLICT'
end

local outbox_base = 8 + write_count * 2
local new_outbox = 0
for index = 1, outbox_count do
  local item_id = ARGV[outbox_base + (index - 1) * 2 + 1]
  local item_doc = ARGV[outbox_base + (index - 1) * 2 + 2]
  local key = KEYS[6 + write_count + index]
  local existing = redis.call('GET', key)
  if existing and existing ~= item_doc then
    return 'OUTBOX_CONFLICT'
  end
  if not existing then
    new_outbox = new_outbox + 1
  end
  if not item_id or not item_doc then
    return 'CORRUPT_ARGUMENTS'
  end
end
local attempted_outbox = redis.call('SCARD', KEYS[5]) + new_outbox
if attempted_outbox > max_pending_outbox then
  return 'OUTBOX_CAPACITY:' .. tostring(attempted_outbox)
end

for index = 1, write_count do
  local kind = ARGV[7 + index * 2]
  local post_image = ARGV[8 + index * 2]
  local key = KEYS[6 + index]
  if kind == 'string_set' then
    local current = redis.call('GET', key)
    if current and (not tonumber(current) or tonumber(current) > tonumber(post_image)) then
      return 'ALLOCATOR_BEHIND'
    end
  end
end

for index = 1, write_count do
  local kind = ARGV[7 + index * 2]
  local post_image = ARGV[8 + index * 2]
  local key = KEYS[6 + index]
  if kind == 'json_set' then
    redis.call('JSON.SET', key, '$', post_image)
  elseif kind == 'string_set' then
    redis.call('SET', key, post_image)
  elseif kind == 'delete' then
    redis.call('DEL', key)
  else
    return 'CORRUPT_ARGUMENTS'
  end
end

-- The write loop above and this readback are one script. Redis runs an EVAL to
-- completion with nothing interleaved, and AuthoritativeBundle::build refuses a
-- bundle that writes the same replay target twice, so no key read here can hold
-- anything except what the loop above put in it. That leaves exactly one
-- meaning per kind for a disagreement, and only two of them are faults:
--
--   json_set   -- RedisJSON re-serializing the document it was just handed:
--                key order, number spelling, escapes. The write landed, so
--                refusing would strand a durably applied post-image with no
--                witness and no way for a retry to get past it. This arm only
--                *observes*: it records the first index that disagreed, the
--                script finishes normally, and the caller is told through
--                'FINALIZED_RENORMALISED:<index>'. Catching a genuine clobber
--                is checkpoint's job (checkpoint_authoritative.lua re-reads
--                targets other writers have had time to touch) and boot
--                reconcile's; there is no clobber for *this* readback to catch.
--   string_set -- GET returns the exact bytes SET stored.
--   delete     -- EXISTS after DEL is exact.
--
-- Neither of the last two can renormalize anything, so both still refuse. A
-- refusal names the replay index it caught, so the caller can resolve it
-- against the plan it sent and refuse by key rather than by whole bundle.
local renormalised = nil
for index = 1, write_count do
  local kind = ARGV[7 + index * 2]
  local post_image = ARGV[8 + index * 2]
  local key = KEYS[6 + index]
  if kind == 'json_set' then
    if renormalised == nil and redis.call('JSON.GET', key) ~= post_image then
      renormalised = index
    end
  elseif kind == 'string_set' then
    if redis.call('GET', key) ~= post_image then
      return 'READBACK_MISMATCH:' .. tostring(index)
    end
  elseif kind == 'delete' then
    if redis.call('EXISTS', key) ~= 0 then
      return 'READBACK_MISMATCH:' .. tostring(index)
    end
  end
end

for index = 1, outbox_count do
  local item_id = ARGV[outbox_base + (index - 1) * 2 + 1]
  local item_doc = ARGV[outbox_base + (index - 1) * 2 + 2]
  local key = KEYS[6 + write_count + index]
  if redis.call('EXISTS', key) == 0 then
    redis.call('SET', key, item_doc)
  end
  redis.call('SADD', KEYS[5], item_id)
end

redis.call('SET', KEYS[6], ARGV[8])
redis.call('SET', KEYS[2], ARGV[2])
redis.call('SREM', KEYS[4], ARGV[4])
if renormalised ~= nil then
  return 'FINALIZED_RENORMALISED:' .. tostring(renormalised)
end
return 'FINALIZED'
