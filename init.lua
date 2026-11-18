hs.autoLaunch(true)

-- ⌥⌘1 / ⌥⌘2 (win+alt no teclado Logitech): entrada do monitor + canal dos periféricos.
for _, n in ipairs({ "1", "2" }) do
  hs.hotkey.bind({ "cmd", "alt" }, n, function()
    hs.task.new(hs.configdir .. "/kvm-switch.sh", function(code, _, err)
      if code ~= 0 then hs.notify.show("KVM", "", err) end
    end, { n }):start()
  end)
end
