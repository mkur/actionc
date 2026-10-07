"""Current native-v2 physical map checks for the frozen hosted consumer.

These checks mirror Image::verify's home geometry and stack accounting. Maps
do not serialize live intervals: interference and call preservation remain
compiler-owned proofs. The frozen historical validator is not edited.
"""


def require(condition, message):
    if not condition:
        raise ValueError(message)


def natural(value, label):
    require(type(value) is int and value >= 0, 'Invalid frame-map '+label)
    return value


def validate(routines, fields, pointers):
    start = fields['scratch']['offset'] + 32
    end = fields['scratch']['offset'] + fields['scratch']['size']
    require((start, end, tuple(pointers)) == (0xa0, 0xc0, (0x80, 0x83, 0x86)),
            'Frame-map consumer requires the qualified native-v2 scratch ABI')
    require(isinstance(routines, list), 'Invalid routine maps')
    ids = set()
    for routine in routines:
        required = {'id', 'fixed_frame', 'spill_bytes', 'arguments', 'objects',
                    'temporaries', 'calls', 'local_stack_peak', 'whole_task_stack_bound'}
        require(isinstance(routine, dict) and required <= routine.keys(), 'Incomplete routine map')
        identity = natural(routine['id'], 'routine identity')
        require(identity not in ids, 'Duplicate routine map')
        ids.add(identity)
        frame = natural(routine['fixed_frame'], 'fixed frame')
        require(frame <= 254 and frame % 2 == 0 and
                natural(routine['spill_bytes'], 'spill extent') <= frame and
                routine['whole_task_stack_bound'] is None, 'Invalid fixed frame')
        for field in ('arguments', 'objects', 'temporaries', 'calls'):
            require(isinstance(routine[field], list), 'Invalid frame-map '+field)
        for argument in routine['arguments']:
            require(isinstance(argument, dict) and
                    {'offset', 'body_displacement', 'size'} <= argument.keys(), 'Invalid incoming map')
            offset = natural(argument['offset'], 'incoming offset')
            size = natural(argument['size'], 'incoming size')
            displacement = natural(argument['body_displacement'], 'incoming displacement')
            require(size > 0 and displacement == frame + 4 + offset and
                    displacement + size <= 256, 'Invalid incoming displacement')
        extents = []
        for obj in routine['objects']:
            require(isinstance(obj, dict) and {'displacement', 'size'} <= obj.keys(), 'Invalid frame object')
            extents.append((obj['displacement'], obj['size']))
        temps, legacy_pool = set(), None
        for temp in routine['temporaries']:
            require(isinstance(temp, dict) and set(temp) == {'id', 'size', 'home'}, 'Invalid temporary')
            identity = natural(temp['id'], 'temporary identity')
            size = natural(temp['size'], 'temporary size')
            require(identity not in temps and 1 <= size <= 4, 'Invalid temporary identity/width')
            temps.add(identity)
            home = temp['home']
            require(isinstance(home, dict), 'Invalid temporary home')
            if home.get('kind') == 'stack':
                require(set(home) == {'kind', 'displacement'}, 'Invalid stack home')
                extents.append((home['displacement'], size))
            elif home.get('kind') == 'direct_page':
                require(set(home) == {'kind', 'offset'}, 'Invalid direct-page home')
                offset = natural(home['offset'], 'direct-page offset')
                legacy = size == 3 and offset in pointers
                resident = size in (2, 3) and offset % 2 == 0 and start <= offset and offset + size <= end
                require((legacy or resident) and (not legacy or not routine['calls']) and
                        legacy_pool in (None, legacy), 'Invalid direct-page temporary map')
                legacy_pool = legacy
            else:
                raise ValueError('Unknown temporary home')
        # Different temporaries may share physical homes at disjoint live ranges.
        # A physical artifact map alone cannot establish or refute that proof.
        for offset, size in extents:
            offset, size = natural(offset, 'object offset'), natural(size, 'object size')
            require(offset > 0 and size > 0 and offset + size <= frame + 1,
                    'Frame map object exceeds its allocation')
        call_peak = 0
        for call in routine['calls']:
            require(isinstance(call, dict) and set(call) == {'outgoing', 'transfer_peak'}, 'Invalid call map')
            outgoing = natural(call['outgoing'], 'outgoing extent')
            transfer = natural(call['transfer_peak'], 'transfer peak')
            require(0 < outgoing <= 255 and outgoing % 2 == 1 and transfer in (3, 6), 'Invalid call extent')
            call_peak = max(call_peak, outgoing + transfer)
        peak = natural(routine['local_stack_peak'], 'local peak')
        require(peak == frame + call_peak, 'Invalid local stack cost map')


def install():
    """Install a versioned consumer overlay before native_program validates maps."""
    import native_frame_maps
    from native_abi import FIELDS, POINTERS
    native_frame_maps.validate = lambda routines: validate(routines, FIELDS, POINTERS)
