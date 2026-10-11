(component
  (type $interface (instance (export "forbidden" (func))))
  (import "wasi:clocks/wall-clock@0.2.0" (instance (type $interface)))
)
