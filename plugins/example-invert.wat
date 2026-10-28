(module
  (func (export "dudel_api_version") (result i32)
    i32.const 1)
  ;; Low bytes are R, G, B; the high byte is alpha.
  (func (export "dudel_filter_rgba") (param $rgba i32) (result i32)
    local.get $rgba
    i32.const 0x00ffffff
    i32.xor))
