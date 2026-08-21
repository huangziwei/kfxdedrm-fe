--[[--
What every `*_spec.lua` opens with: the search path, the KOReader stubs, and
three assertions.

`spec/run.sh` sets `KFXDEDRM_SPEC` to this directory. Requiring this module
puts the plugin and the spec folder on `package.path`, so a spec can
`require("lib.engine")` exactly as `main.lua` does.
]]

local harness = {}

harness.SPEC = os.getenv("KFXDEDRM_SPEC")
    or error("KFXDEDRM_SPEC is not set -- run these through spec/run.sh")
harness.PLUGIN = harness.SPEC:gsub("/spec$", "") .. "/kfxdedrm.koplugin"

package.path = harness.PLUGIN .. "/?.lua;" .. harness.SPEC .. "/?.lua;" .. package.path
require("stubs")

local passed, failed = 0, 0

--- One assertion. `detail` is printed only when it fails.
function harness.check(name, ok, detail)
    if ok then
        passed = passed + 1
    else
        failed = failed + 1
        print("FAIL  " .. name .. (detail and ("  -- " .. tostring(detail)) or ""))
    end
end

function harness.eq(name, got, want)
    harness.check(name, got == want, string.format("got %s, want %s", tostring(got), tostring(want)))
end

function harness.eqlist(name, got, want)
    local ok = #got == #want
    if ok then
        for i = 1, #want do
            if got[i] ~= want[i] then
                ok = false
                break
            end
        end
    end
    harness.check(name, ok,
        "got [" .. table.concat(got, ", ") .. "] want [" .. table.concat(want, ", ") .. "]")
end

--- The line a spec ends on, and its exit status.
function harness.report()
    print(string.format("%d passed, %d failed", passed, failed))
    os.exit(failed == 0 and 0 or 1)
end

return harness
