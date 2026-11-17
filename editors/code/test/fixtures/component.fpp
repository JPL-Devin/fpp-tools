module Fw {
  port Cmd
  port Tlm
  port Log
}
module Demo {
  active component Widget {
    async input port cmdIn: Fw.Cmd
    guarded input port tlmIn: Fw.Tlm
    sync input port logIn: Fw.Log
    output port tlmOut: Fw.Tlm
    output port logOut: [2] Fw.Log
  }
}
