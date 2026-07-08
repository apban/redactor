--- === Redactor ===
---
--- Click-drag black boxes to cover sensitive content, then take a screenshot
--- to clipboard. Boxes vanish automatically once the image hits the clipboard.
---
--- Download: https://github.com/apban/redactor

local obj    = {}
obj.__index  = obj

-- Metadata
obj.name     = "Redactor"
obj.version  = "0.1.0"
obj.author   = "Adam Bankhead"
obj.homepage = "https://github.com/apban/redactor"
obj.license  = "MIT - https://opensource.org/licenses/MIT"

-- ===== Configuration =====

--- Redactor.boxLevel
--- Variable
--- Window level for committed black boxes. Defaults to screenSaver. If your
--- screenshot UI dims the boxes during capture, bump this to
--- `hs.canvas.windowLevels.assistiveTechHigh`.
obj.boxLevel = hs.canvas.windowLevels.screenSaver

--- Redactor.overlayLevel
--- Variable
--- Window level for the slight tint shown during draw mode.
obj.overlayLevel = hs.canvas.windowLevels.overlay

--- Redactor.watchTimeout
--- Variable
--- Seconds before the clipboard watcher gives up if no screenshot is taken.
obj.watchTimeout = 120

--- Redactor.screenshotPassthroughKey
--- Variable
--- Key (string) that should exit draw mode and pass through to your system
--- screenshot tool. Default "s" for ⌘⇧S. Set to nil to disable passthrough.
obj.screenshotPassthroughKey = "s"

--- Redactor.screenshotPassthroughMods
--- Variable
--- Modifier flags for the passthrough key. Map of {cmd=true, shift=true, ...}.
obj.screenshotPassthroughMods = { cmd = true, shift = true }

-- ===== Internal state =====
obj._boxes      = nil
obj._overlays   = nil
obj._drawMode   = false
obj._currentBox = nil
obj._startPoint = nil
obj._eventTap   = nil
obj._pbWatcher  = nil
obj._pbTimeout  = nil
obj._pbInitial  = nil

function obj:init()
  self._boxes    = {}
  self._overlays = {}
  return self
end

-- ===== Internal helpers =====

function obj:_clearBoxes()
  for _, b in ipairs(self._boxes) do b:delete() end
  self._boxes = {}
end

function obj:_stopWatcher()
  if self._pbWatcher then self._pbWatcher:stop(); self._pbWatcher = nil end
  if self._pbTimeout then self._pbTimeout:stop(); self._pbTimeout = nil end
  self._pbInitial = nil
end

function obj:_startWatcher()
  self:_stopWatcher()
  self._pbInitial = hs.pasteboard.changeCount()
  self._pbWatcher = hs.timer.doEvery(0.2, function()
    local now = hs.pasteboard.changeCount()
    if now ~= self._pbInitial then
      if hs.pasteboard.readImage() then
        self:_clearBoxes()
        self:_stopWatcher()
      else
        -- Clipboard changed but it's not an image (user copied text, etc).
        -- Re-baseline and keep waiting for the actual screenshot.
        self._pbInitial = now
      end
    end
  end)
  self._pbTimeout = hs.timer.doAfter(self.watchTimeout, function()
    self:_stopWatcher()
  end)
end

function obj:_teardownOverlay()
  for _, o in ipairs(self._overlays) do o:delete() end
  self._overlays = {}
end

function obj:_exitDrawMode(commit)
  if not self._drawMode then return end
  self._drawMode = false
  if self._eventTap then self._eventTap:stop(); self._eventTap = nil end
  self:_teardownOverlay()
  if self._currentBox then self._currentBox:delete(); self._currentBox = nil end
  self._startPoint = nil
  if not commit then
    self:_clearBoxes()
    return
  end
  if #self._boxes > 0 then self:_startWatcher() end
end

function obj:_matchesPassthrough(key, mods)
  if not self.screenshotPassthroughKey then return false end
  if key ~= self.screenshotPassthroughKey then return false end
  local want = self.screenshotPassthroughMods or {}
  for _, m in ipairs({ "cmd", "shift", "alt", "ctrl", "fn" }) do
    if (want[m] or false) ~= (mods[m] or false) then return false end
  end
  return true
end

function obj:_enterDrawMode()
  if self._drawMode then return end
  self:_stopWatcher()
  self._drawMode = true

  for _, screen in ipairs(hs.screen.allScreens()) do
    local o = hs.canvas.new(screen:fullFrame())
    o:appendElements({
      type = "rectangle",
      action = "fill",
      fillColor = { red = 0, green = 0, blue = 0, alpha = 0.08 },
    })
    o:level(self.overlayLevel)
    o:behavior({ "canJoinAllSpaces", "stationary" })
    o:show()
    table.insert(self._overlays, o)
  end

  self._eventTap = hs.eventtap.new({
    hs.eventtap.event.types.leftMouseDown,
    hs.eventtap.event.types.leftMouseDragged,
    hs.eventtap.event.types.leftMouseUp,
    hs.eventtap.event.types.keyDown,
  }, function(e)
    local t = e:getType()

    if t == hs.eventtap.event.types.keyDown then
      local key = hs.keycodes.map[e:getKeyCode()]
      local mods = e:getFlags()
      if key == "escape" then
        self:_exitDrawMode(false)
        return true
      elseif key == "return" or key == "padenter" then
        self:_exitDrawMode(true)
        return true
      elseif self:_matchesPassthrough(key, mods) then
        -- Commit boxes and let the keystroke pass through to the system
        -- screenshot tool.
        self:_exitDrawMode(true)
        return false
      end
      return false
    end

    local loc = e:location()
    local p = { x = loc.x, y = loc.y }

    if t == hs.eventtap.event.types.leftMouseDown then
      self._startPoint = p
      self._currentBox = hs.canvas.new({ x = p.x, y = p.y, w = 1, h = 1 })
      self._currentBox:appendElements({
        type = "rectangle",
        action = "fill",
        fillColor = { red = 0, green = 0, blue = 0, alpha = 1 },
      })
      self._currentBox:level(self.boxLevel)
      self._currentBox:behavior({ "canJoinAllSpaces", "stationary" })
      self._currentBox:show()
      return true

    elseif t == hs.eventtap.event.types.leftMouseDragged
           and self._currentBox and self._startPoint then
      local x = math.min(self._startPoint.x, p.x)
      local y = math.min(self._startPoint.y, p.y)
      local w = math.max(1, math.abs(p.x - self._startPoint.x))
      local h = math.max(1, math.abs(p.y - self._startPoint.y))
      self._currentBox:frame({ x = x, y = y, w = w, h = h })
      return true

    elseif t == hs.eventtap.event.types.leftMouseUp and self._currentBox then
      local f = self._currentBox:frame()
      if f.w >= 4 and f.h >= 4 then
        table.insert(self._boxes, self._currentBox)
      else
        self._currentBox:delete()
      end
      self._currentBox = nil
      self._startPoint = nil
      return true
    end

    return false
  end)
  self._eventTap:start()
end

-- ===== Public API =====

--- Redactor:toggleDrawMode()
--- Method
--- Enter draw mode if inactive; exit and commit if active.
function obj:toggleDrawMode()
  if self._drawMode then
    self:_exitDrawMode(true)
  else
    self:_enterDrawMode()
  end
end

--- Redactor:panicClear()
--- Method
--- Immediately remove all boxes and stop the clipboard watcher.
function obj:panicClear()
  self:_exitDrawMode(false)
  self:_clearBoxes()
  self:_stopWatcher()
  hs.alert.show("Redactor: cleared")
end

--- Redactor:bindHotkeys(mapping)
--- Method
--- Binds hotkeys for Redactor. Spec keys:
---   * toggle - enter/exit draw mode
---   * panic  - clear all boxes immediately
---
--- Example:
---   spoon.Redactor:bindHotkeys({
---     toggle = { {"cmd", "alt"}, "b" },
---     panic  = { {"cmd", "alt", "shift"}, "b" },
---   })
function obj:bindHotkeys(mapping)
  local spec = {
    toggle = hs.fnutils.partial(self.toggleDrawMode, self),
    panic  = hs.fnutils.partial(self.panicClear, self),
  }
  hs.spoons.bindHotkeysToSpec(spec, mapping)
  return self
end

return obj
