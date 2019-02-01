use byteorder::{BigEndian, ReadBytesExt};

pub(crate) fn scripts(parent_data: &[u8], offset_data: &[u8], num: usize) -> Vec<Script> {
    let mut result = vec!();
    for i in 0..num {
        let offset = (&offset_data[i * 4..]).read_i32::<BigEndian>().unwrap() as usize;
        result.push(new_script(parent_data, offset));
    }
    result
}

/// finds any scripts that are pointed to by Goto's and Subroutines but dont exist yet.
pub(crate) fn fragment_scripts(parent_data: &[u8], action_scripts: &[&[Script]]) -> Vec<Script> {
    let mut fragments: Vec<Script> = vec!();
    for scripts in action_scripts.iter() {
        for script in scripts.iter() {
            for event in &script.events {
                if event.namespace == 0 && (event.code == 7 || event.code == 9) { // if the event is a subroutine or goto
                    if let Some(Argument::Offset(offset)) = event.arguments.get(0) {
                        let mut is_action = false;
                        'outer: for check_scripts in action_scripts.iter() {
                            for check_script in check_scripts.iter() {
                                if check_script.offset == *offset as u32{
                                    is_action = true;
                                    break 'outer;
                                }
                            }
                        }
                        let already_added = fragments.iter().any(|x| x.offset == *offset as u32);

                        if !is_action && !already_added {
                            fragments.push(new_script(parent_data, *offset as usize));
                        }
                    }
                }
            }
        }
    }
    fragments
}

fn new_script(parent_data: &[u8], offset: usize) -> Script {
    let events = if offset > 0 && offset < parent_data.len() {
        let mut events = vec!();
        let mut event_offset = offset;
        loop {
            let namespace     = parent_data[event_offset];
            let code          = parent_data[event_offset + 1];
            let num_arguments = parent_data[event_offset + 2];
            let unk1          = parent_data[event_offset + 3];
            let raw_id = (&parent_data[event_offset ..]).read_u32::<BigEndian>().unwrap();

            if code == 0 && namespace == 0 { // seems hacky but its what brawlbox does
                break
            }

            // Dont really understand what FADEF00D or 0xFADE0D8A means but it's apparently added by PSA
            // and brawlbox just skips arguments on events that have one of these ID's
            if raw_id != 0xFADEF00D && raw_id != 0xFADE0D8A {
                let argument_offset = (&parent_data[event_offset + 4 ..]).read_u32::<BigEndian>().unwrap();
                // TODO: This only occurs when called by fragment_scripts triggered by subroutines
                //       Track down which subroutines are pointing at weird data
                //       Looks like the data is offset by 4 bytes, we are getting an argument_offset of 0xFADEF00D, 0x0b000200, 0x60a0800 which are valid events
                if argument_offset as usize >= parent_data.len() {
                    debug!("(raw_id, argument_offset) = (0x{:08x}, 0x{:08x})", raw_id, argument_offset);
                    break
                }
                let arguments = arguments(parent_data, argument_offset as usize, num_arguments as usize);
                events.push(Event {
                    namespace,
                    code,
                    unk1,
                    arguments,
                });
            }

            event_offset += EVENT_SIZE;
        }
        events
    } else {
        vec!()
    };
    Script { events, offset: offset as u32 }
}


fn arguments(parent_data: &[u8], argument_offset: usize, num_arguments: usize) -> Vec<Argument> {
    let mut arguments = vec!();
    for i in 0..num_arguments as usize {
        let argument_offset = argument_offset as usize + i * ARGUMENT_SIZE;
        let ty   = (&parent_data[argument_offset     ..]).read_i32::<BigEndian>().unwrap();
        let data = (&parent_data[argument_offset + 4 ..]).read_i32::<BigEndian>().unwrap();

        let argument = match ty {
            0 => Argument::Value (data),
            1 => Argument::Scalar (data as f32 / 60000.0),
            2 => Argument::Offset (data),
            3 => Argument::Bool (data == 1),
            4 => Argument::File (data),
            5 => {
                let data = data as u32;
                let memory_type    = ((data & 0xF0000000) >> 28) as u8;
                let data_type      = ((data & 0x0F000000) >> 24) as u8;
                let memory_address =  (data & 0x00FFFFFF)        as u32;

                let memory = VariableMemory::new(memory_type, memory_address);
                let data_type = VariableDataType::new(data_type);

                Argument::Variable (Variable {memory, data_type})
            }
            6 => Requirement::new(data),
            _ => Argument::Unknown (ty, data),
        };
        arguments.push(argument);
    }

    arguments
}

#[derive(Clone, Debug)]
pub struct Script {
    pub events: Vec<Event>,
    pub offset: u32
}

// Events are like lines of code in a script
const EVENT_SIZE: usize = 0x8;
#[derive(Serialize, Clone, Debug)]
pub struct Event {
    pub namespace: u8,
    pub code: u8,
    pub unk1: u8,
    pub arguments: Vec<Argument>,
}

impl Event {
    pub fn raw_id(&self) -> u32 {
        let num_args = self.arguments.len();
        assert!(num_args < 0x100);
        (self.namespace as u32) << 24 | (self.code as u32) << 16 | (num_args as u32) << 8
    }
}

const ARGUMENT_SIZE: usize = 0x8;
#[derive(Serialize, Clone, Debug)]
pub enum Argument {
    Value (i32),
    Scalar (f32),
    Offset (i32),
    Bool (bool),
    File (i32),
    Variable (Variable),
    Requirement { flip: bool, ty: Requirement },
    Unknown (i32, i32)
}

#[derive(Serialize, Clone, Debug)]
pub struct Variable {
    pub memory: VariableMemory,
    pub data_type: VariableDataType,
}

#[derive(Serialize, Clone, Debug)]
pub enum VariableMemory {
    /// Known as IC in existing tools
    InternalConstant (InternalConstant),
    /// Known as LA in existing tools
    LongtermAccess (u32),
    /// Known as RA in existing tools
    RandomAccess (u32),
    Unknown { memory_type: u8, memory_address: u32 }
}

impl VariableMemory {
    fn new(memory_type: u8, memory_address: u32) -> VariableMemory {
        match memory_type {
            0 => VariableMemory::InternalConstant (InternalConstant::new(memory_address)),
            1 => VariableMemory::LongtermAccess (memory_address),
            2 => VariableMemory::RandomAccess (memory_address),
            _ => VariableMemory::Unknown { memory_type, memory_address }
        }
    }
}

#[derive(Serialize, Clone, Debug)]
pub enum InternalConstant {
    CurrentFrame,
    Damage,
    CharacterXPosition,
    CharacterYPosition,
    CharacterDirection,
    CharacterDirectionOpposite,
    VerticalCharacterVelocity,
    CurrentFrameSpeed,
    HorizontalCharacterVelocity,
    Knockback,
    SurfaceTraction,
    XVelocity,
    LaunchSpeed,

    RightVelocity,
    LeftVelocity,
    UpVelocity,
    DownVelocity,

    ControlStickXAxis,
    ControlStickXAxisRelative,
    ControlStickXAxisRelativeReverse,
    ControlStickXAxisAbsolute,
    ControlStickXAxisReverse,
    ControlStickXAxisReverse2,
    ControlStickYAxis,
    ControlStickYAxisAbsolute,
    ControlStickYAxisReverse,
    ControlStickYAxis2,

    PreviousControlStickXAxis,
    PreviousControlStickXAxisRelative,
    PreviousControlStickXAxisRelativeReverse,
    PreviousControlStickXAxisAbsolute,
    PreviousControlStickXAxisReverse,
    PreviousControlStickXAxisReverse2,
    PreviousControlStickYAxis,
    PreviousControlStickYAxisAbsolute,
    PreviousControlStickYAxisReverse,
    PreviousControlStickYAxis2,

    CurrentSubaction,
    CurrentAction,
    PreviousAction,
    HeldItem,
    EffectOfAttack,

    FramesSinceNormal,
    FramesSinceSpecial,
    FramesSinceJump,
    FramesSinceShield,
    FramesSinceShield2,

    TurnRunFrameTimer,
    JumpStartTimer,
    MaxJumpCount,
    GlideStartTimer,
    TermVelFrameTimer,

    Address (u32)
}

impl InternalConstant {
    fn new(address: u32) -> InternalConstant {
        match address {
           00000 => InternalConstant::CurrentFrame,
           00002 => InternalConstant::Damage,
           00003 => InternalConstant::CharacterXPosition,
           00004 => InternalConstant::CharacterYPosition,
           00008 => InternalConstant::CharacterDirection,
           00009 => InternalConstant::CharacterDirectionOpposite,
           00023 => InternalConstant::VerticalCharacterVelocity,
           00024 => InternalConstant::CurrentFrameSpeed,
           00028 => InternalConstant::HorizontalCharacterVelocity,
           00038 => InternalConstant::Knockback,
           00039 => InternalConstant::SurfaceTraction,
           01000 => InternalConstant::XVelocity,
           01005 => InternalConstant::LaunchSpeed,
           01006 => InternalConstant::RightVelocity,
           01007 => InternalConstant::LeftVelocity,
           01008 => InternalConstant::UpVelocity,
           01009 => InternalConstant::DownVelocity,
           01010 => InternalConstant::ControlStickXAxis,
           01011 => InternalConstant::ControlStickXAxisRelative,
           01012 => InternalConstant::ControlStickXAxisRelativeReverse,
           01013 => InternalConstant::ControlStickXAxisAbsolute,
           01014 => InternalConstant::ControlStickXAxisReverse,
           01017 => InternalConstant::ControlStickXAxisReverse2,
           01018 => InternalConstant::ControlStickYAxis,
           01019 => InternalConstant::ControlStickYAxisAbsolute,
           01020 => InternalConstant::ControlStickYAxisReverse,
           01021 => InternalConstant::ControlStickYAxis2,
           01022 => InternalConstant::PreviousControlStickXAxis,
           01023 => InternalConstant::PreviousControlStickXAxisRelative,
           01024 => InternalConstant::PreviousControlStickXAxisRelativeReverse,
           01025 => InternalConstant::PreviousControlStickXAxisAbsolute,
           01026 => InternalConstant::PreviousControlStickYAxis,
           01027 => InternalConstant::PreviousControlStickYAxisAbsolute,
           01028 => InternalConstant::PreviousControlStickYAxisReverse,
           20000 => InternalConstant::CurrentSubaction,
           20001 => InternalConstant::CurrentAction,
           20003 => InternalConstant::PreviousAction,
           20009 => InternalConstant::HeldItem,
           21004 => InternalConstant::EffectOfAttack,
           21010 => InternalConstant::FramesSinceNormal,
           21012 => InternalConstant::FramesSinceSpecial,
           21014 => InternalConstant::FramesSinceJump,
           21016 => InternalConstant::FramesSinceShield,
           21018 => InternalConstant::FramesSinceShield2,
           23001 => InternalConstant::TurnRunFrameTimer,
           23002 => InternalConstant::JumpStartTimer,
           23003 => InternalConstant::MaxJumpCount,
           23004 => InternalConstant::GlideStartTimer,
           23007 => InternalConstant::TermVelFrameTimer,
            _    => InternalConstant::Address (address)
        }
    }
}

#[derive(Serialize, Clone, Debug)]
pub enum VariableDataType {
    /// Known as Basic in existing tools
    Int,
    /// Known as Float in existing tools
    Float,
    /// Known as Bit in existing tools
    Bool,
    Unknown (u8)
}

impl VariableDataType {
    fn new(value: u8) -> VariableDataType {
        match value {
            0 => VariableDataType::Int,
            1 => VariableDataType::Float,
            2 => VariableDataType::Bool,
            _ => VariableDataType::Unknown (value),
        }
    }
}

#[derive(Serialize, Clone, Debug)]
pub enum Requirement {
    CharacterExists,
    AnimationEnd,
    AnimationHasLooped,
    OnGround,
    InAir,
    HoldingALedge,
    OnAPassableFloor,
    Comparison,
    BoolIsTrue,
    FacingRight,
    FacingLeft,
    HitboxConnects,
    TouchingAFloorWallOrCeiling,
    IsThrowingSomeone,
    ButtonTap,
    EnteringOrIsInHitLag,
    ArticleExists,
    HasAFloorBelowThePlayer,
    ChangeInAirGroundState,
    ArticleAvailable,
    HoldingItem,
    HoldingItemOfType,
    LightItemIsInGrabRange,
    HeavyItemIsInGrabRange,
    ItemOfTypeIsInGrabbingRange,
    TurningWithItem,
    InWater,
    RollADie,
    SubactionExists,
    ButtonMashingOrStatusExpiredSleepBuryFreeze,
    IsNotInDamagingLens,
    ButtonPress,
    ButtonRelease,
    ButtonPressed,
    ButtonNotPressed,
    StickDirectionPressed,
    StickDirectionNotPressed,
    IsBeingThrownBySomeone1,
    IsBeingThrownBySomeone2,
    HasntTethered3Times,
    HasPassedOverAnEdgeForward,
    HasPassedOverAnEdgeBackward,
    IsHoldingSomeoneInGrab,
    HitboxHasConnected,
    PickUpItem,
    HitByCapeEffect,
    InWalljump,
    InWallCling,
    InFootstoolRange,
    IsFallingOrHitDown,
    HasSmashBall,
    CanPickupAnotherItem,
    FSmashShorcut,
    TapJumpOn,
    Unknown (i32)
}

impl Requirement {
    fn new(value: i32) -> Argument {
        let flip = value >> 31 == 1;
        let ty = match value & 0xFF {
              0 => Requirement::CharacterExists,
              1 => Requirement::AnimationEnd,
              2 => Requirement::AnimationHasLooped,
              3 => Requirement::OnGround,
              4 => Requirement::InAir,
              5 => Requirement::HoldingALedge,
              6 => Requirement::OnAPassableFloor,
              7 => Requirement::Comparison,
              8 => Requirement::BoolIsTrue,
              9 => Requirement::FacingRight,
             10 => Requirement::FacingLeft,
             11 => Requirement::HitboxConnects,
             12 => Requirement::TouchingAFloorWallOrCeiling,
             13 => Requirement::IsThrowingSomeone,
             15 => Requirement::ButtonTap,
             20 => Requirement::EnteringOrIsInHitLag,
             21 => Requirement::ArticleExists,
             23 => Requirement::HasAFloorBelowThePlayer,
             27 => Requirement::ChangeInAirGroundState,
             28 => Requirement::ArticleAvailable,
             31 => Requirement::HoldingItem,
             32 => Requirement::HoldingItemOfType,
             33 => Requirement::LightItemIsInGrabRange,
             34 => Requirement::HeavyItemIsInGrabRange,
             35 => Requirement::ItemOfTypeIsInGrabbingRange,
             36 => Requirement::TurningWithItem,
             42 => Requirement::InWater,
             43 => Requirement::RollADie,
             44 => Requirement::SubactionExists,
             46 => Requirement::ButtonMashingOrStatusExpiredSleepBuryFreeze,
             47 => Requirement::IsNotInDamagingLens,
             48 => Requirement::ButtonPress,
             49 => Requirement::ButtonRelease,
             50 => Requirement::ButtonPressed,
             51 => Requirement::ButtonNotPressed,
             52 => Requirement::StickDirectionPressed,
             53 => Requirement::StickDirectionNotPressed,
             55 => Requirement::IsBeingThrownBySomeone1,
             56 => Requirement::IsBeingThrownBySomeone2,
             57 => Requirement::HasntTethered3Times,
             58 => Requirement::HasPassedOverAnEdgeForward,
             59 => Requirement::HasPassedOverAnEdgeBackward,
             60 => Requirement::IsHoldingSomeoneInGrab,
             61 => Requirement::HitboxHasConnected,
             71 => Requirement::PickUpItem,
             76 => Requirement::HitByCapeEffect,
            103 => Requirement::InWalljump,
            104 => Requirement::InWallCling,
            105 => Requirement::InFootstoolRange,
            108 => Requirement::IsFallingOrHitDown,
            109 => Requirement::HasSmashBall,
            111 => Requirement::CanPickupAnotherItem,
            115 => Requirement::FSmashShorcut,
            123 => Requirement::TapJumpOn,
            v   => Requirement::Unknown (v),
        };
        Argument::Requirement { ty, flip }
    }
}
