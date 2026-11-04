-- Fills calendar.aegle with this month, weeks starting on Monday.

local now = os.date("*t")
local first = os.date("*t", os.time({ year = now.year, month = now.month, day = 1 }))
local last = os.date("*t", os.time({ year = now.year, month = now.month + 1, day = 0 })).day

local days = {}
for _, name in { "Mo", "Tu", "We", "Th", "Fr", "Sa", "Su" } do
    table.insert(days, { label = name })
end
for _ = 1, (first.wday + 5) % 7 do
    table.insert(days, { label = "" })
end
for day = 1, last do
    table.insert(days, { label = tostring(day), today = day == now.day })
end

for index, day in days do
    day.index = index
end

state.month = os.date("%B %Y")
state.days = days
