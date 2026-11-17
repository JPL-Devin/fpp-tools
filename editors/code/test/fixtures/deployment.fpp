# A notional deployment that exists only as an FPP design: no component here has
# an implementation. It imports two subtopologies and adds its own instances.
module Fw {
  port Cmd
  port CmdReg
  port CmdResponse
  port Com
  port Log
  port Time
  port Sched
  port Buffer
}

module Cdh {
  active component Dispatcher {
    async input port seqCmdBuff: [2] Fw.Com
    async input port compCmdStat: Fw.CmdResponse
    guarded input port compCmdReg: [8] Fw.CmdReg
    output port compCmdSend: [8] Fw.Cmd
  }
  active component EventLog {
    async input port LogRecv: Fw.Log
    output port PktSend: Fw.Com
  }
  passive component Clock {
    sync input port timeGetPort: Fw.Time
  }

  instance dispatcher: Dispatcher base id 0x1000 queue size 20
  instance eventLog: EventLog base id 0x1100 queue size 20
  instance clock: Clock base id 0x1200

  topology Subtopology {
    instance dispatcher
    instance eventLog
    instance clock

    port commandIn = dispatcher.seqCmdBuff
    port eventsOut = eventLog.PktSend
  }
}

module Comms {
  passive component Framer {
    guarded input port comIn: [2] Fw.Com
    output port framedOut: Fw.Buffer
    sync input port framedIn: Fw.Buffer
    output port comOut: Fw.Com
  }
  active component Radio {
    async input port txIn: Fw.Buffer
    output port rxOut: Fw.Buffer
    sync input port run: Fw.Sched
  }

  instance framer: Framer base id 0x2000
  instance radio: Radio base id 0x2100 queue size 10

  topology Subtopology {
    instance framer
    instance radio

    port downlink = framer.comIn
    port uplink = framer.comOut
    port radioRun = radio.run

    connections Link {
      framer.framedOut -> radio.txIn
      radio.rxOut -> framer.framedIn
    }
  }
}

module Rover {
  passive component RateGroup {
    sync input port CycleIn: Fw.Sched
    output port RateGroupMemberOut: [4] Fw.Sched
  }
  passive component Timer {
    output port CycleOut: Fw.Sched
  }
  active component Imager {
    async input port run: Fw.Sched
    async input port cmdIn: Fw.Cmd
    output port cmdRegOut: Fw.CmdReg
    output port cmdResponseOut: Fw.CmdResponse
    output port imageOut: Fw.Com
    output port logOut: Fw.Log
    time get port timeCaller
    internal port exposureDone
  }
  queued component Mobility {
    sync input port run: Fw.Sched
    async input port cmdIn: Fw.Cmd
    output port cmdRegOut: Fw.CmdReg
    output port cmdResponseOut: Fw.CmdResponse
    output port motorOut: [6] Fw.Buffer
    output port logOut: Fw.Log
    time get port timeCaller
  }

  instance timer: Timer base id 0x3000
  instance rateGroup: RateGroup base id 0x3100
  instance imager: Imager base id 0x3200 queue size 10
  instance mobility: Mobility base id 0x3300 queue size 10

  topology Rover {
    import Cdh.Subtopology
    import Comms.Subtopology

    instance timer
    instance rateGroup
    instance imager
    instance mobility

    time connections instance Cdh.clock

    connections RateGroups {
      timer.CycleOut -> rateGroup.CycleIn
      rateGroup.RateGroupMemberOut[0] -> imager.run
      rateGroup.RateGroupMemberOut[1] -> mobility.run
      rateGroup.RateGroupMemberOut[2] -> Comms.Subtopology.radioRun
    }

    connections Commanding {
      Comms.Subtopology.uplink -> Cdh.Subtopology.commandIn[0]
      Cdh.dispatcher.compCmdSend[0] -> imager.cmdIn
      Cdh.dispatcher.compCmdSend[1] -> mobility.cmdIn
      imager.cmdRegOut -> Cdh.dispatcher.compCmdReg[0]
      mobility.cmdRegOut -> Cdh.dispatcher.compCmdReg[1]
      imager.cmdResponseOut -> Cdh.dispatcher.compCmdStat
      mobility.cmdResponseOut -> Cdh.dispatcher.compCmdStat
    }

    connections Downlink {
      Cdh.Subtopology.eventsOut -> Comms.Subtopology.downlink[0]
      imager.imageOut -> Comms.Subtopology.downlink[1]
      imager.logOut -> Cdh.eventLog.LogRecv
      mobility.logOut -> Cdh.eventLog.LogRecv
    }
  }
}
