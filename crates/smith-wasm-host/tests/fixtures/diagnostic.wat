(component
  (core module $m
    (func (export "answer") (result i32) i32.const 42))
  (core instance $i (instantiate $m))
  (func (export "answer") (result u32)
    (canon lift (core func $i "answer"))))
