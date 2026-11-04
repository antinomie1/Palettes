-- Runs while stats.aegle is open: CPU and memory use from /proc, every two
-- seconds, into the document's `cpu` and `memory` states.

local idle_before, total_before = 0, 0

local function sample()
    local cpu = (read("/proc/stat") or ""):match("^cpu%s+([^\n]+)")
    if cpu then
        local fields = {}
        for n in cpu:gmatch("%d+") do
            table.insert(fields, tonumber(n))
        end
        local idle, total = fields[4] + (fields[5] or 0), 0
        for _, n in fields do
            total += n
        end
        if total > total_before and total_before > 0 then
            state.cpu = 1 - (idle - idle_before) / (total - total_before)
        end
        idle_before, total_before = idle, total
    end
    local memory = read("/proc/meminfo") or ""
    local all = tonumber(memory:match("MemTotal:%s+(%d+)"))
    local free = tonumber(memory:match("MemAvailable:%s+(%d+)"))
    if all and free then
        state.memory = 1 - free / all
    end
end

sample()
every(2, sample)
