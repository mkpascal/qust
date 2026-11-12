// Measures the page as rendered: every element's box and the computed styles that define its look,
// its text runs, and the running animations. Evaluated in the page after the screenshot. Defines
// window.__refCollect(), window.__refStyle(element, pseudo) and window.__refName(element).
(function () {
    const names = new Map();

    function round(value, digits = 2) {
        const factor = 10 ** digits;
        return Math.round(value * factor) / factor;
    }

    // "rgb(1, 2, 3)", "rgba(1, 2, 3, 0.5)" or "color(srgb 0.1 0.2 0.3 / 0.5)" (what color-mix()
    // computes to) as [r, g, b, a], r g b 0-255 and a 0-1.
    function parseColor(text) {
        let match = /^rgba?\(([^)]+)\)$/.exec(text);
        if (match) {
            const parts = match[1].split(/[\s,/]+/).filter(Boolean).map(Number);
            return [round(parts[0], 3), round(parts[1], 3), round(parts[2], 3), parts.length > 3 ? round(parts[3], 4) : 1];
        }
        match = /^color\(srgb ([^)]+)\)$/.exec(text);
        if (match) {
            const [rgb, alpha] = match[1].split('/');
            const parts = rgb.trim().split(/\s+/).map(Number);
            return [round(parts[0] * 255, 3), round(parts[1] * 255, 3), round(parts[2] * 255, 3), alpha === undefined ? 1 : round(Number(alpha), 4)];
        }
        return null;
    }

    // Splits a comma list at top level (not inside parentheses).
    function splitList(text) {
        const out = [];
        let depth = 0;
        let start = 0;
        for (let i = 0; i < text.length; i++) {
            const c = text[i];
            if (c === '(') depth++;
            else if (c === ')') depth--;
            else if (c === ',' && depth === 0) {
                out.push(text.slice(start, i).trim());
                start = i + 1;
            }
        }
        out.push(text.slice(start).trim());
        return out;
    }

    function parseShadows(text) {
        if (text === 'none') return null;
        return splitList(text).map((shadow) => {
            const colorMatch = /(rgba?\([^)]*\)|color\([^)]*\))/.exec(shadow);
            const color = colorMatch ? parseColor(colorMatch[1]) : null;
            const rest = shadow.replace(colorMatch ? colorMatch[1] : '', '').trim();
            const inset = /\binset\b/.test(rest);
            const lengths = rest.replace('inset', '').trim().split(/\s+/).filter(Boolean).map((part) => parseFloat(part));
            const [x = 0, y = 0, blur = 0, spread = 0] = lengths;
            return { inset, x, y, blur, spread, color };
        });
    }

    function seconds(text) {
        return splitList(text).map((part) => round(parseFloat(part) * (part.endsWith('ms') ? 1 : 1000), 3));
    }

    const px = (text) => (/^-?[\d.e+-]+px$/.test(text) ? round(parseFloat(text), 3) : text);

    const TEXT_PROPS = ['font-family', 'font-size', 'font-weight', 'font-style', 'line-height', 'letter-spacing', 'font-variant-numeric', 'text-transform', 'text-decoration-line', 'text-align', 'white-space', 'text-overflow'];
    const BOX_PROPS = [
        'display', 'position', 'top', 'right', 'bottom', 'left', 'z-index', 'overflow-x', 'overflow-y',
        'flex-direction', 'flex-wrap', 'justify-content', 'align-items', 'align-self', 'flex-grow', 'flex-shrink', 'flex-basis',
        'grid-template-columns', 'grid-template-rows', 'row-gap', 'column-gap',
        'width', 'height', 'min-width', 'min-height', 'max-width',
        'padding-top', 'padding-right', 'padding-bottom', 'padding-left',
        'margin-top', 'margin-right', 'margin-bottom', 'margin-left',
    ];
    const LOOK_PROPS = [
        'color', 'background-color', 'background-image', 'background-clip',
        'border-top-width', 'border-right-width', 'border-bottom-width', 'border-left-width',
        'border-top-style', 'border-right-style', 'border-bottom-style', 'border-left-style',
        'border-top-color', 'border-right-color', 'border-bottom-color', 'border-left-color',
        'border-top-left-radius', 'border-top-right-radius', 'border-bottom-right-radius', 'border-bottom-left-radius',
        'box-shadow', 'opacity', 'outline-style', 'outline-width', 'outline-color', 'outline-offset',
        'transform', 'transform-origin', 'mask-image', 'fill', 'stroke', 'visibility', 'cursor', 'image-rendering', 'user-select', 'pointer-events', 'content',
    ];
    const MOTION_PROPS = [
        'transition-property', 'transition-duration', 'transition-timing-function', 'transition-delay',
        'animation-name', 'animation-duration', 'animation-timing-function', 'animation-delay', 'animation-iteration-count', 'animation-direction', 'animation-fill-mode', 'animation-play-state',
    ];

    // Computed values that are the property's initial value (omitted from the output).
    const DEFAULTS = {
        position: 'static', 'z-index': 'auto', 'overflow-x': 'visible', 'overflow-y': 'visible',
        'flex-wrap': 'nowrap', 'justify-content': 'normal', 'align-items': 'normal', 'align-self': 'auto',
        'flex-grow': '0', 'flex-shrink': '1', 'flex-basis': 'auto', 'grid-template-columns': 'none', 'grid-template-rows': 'none',
        'row-gap': 'normal', 'column-gap': 'normal', 'min-width': 'auto', 'min-height': 'auto', 'max-width': 'none',
        'padding-top': '0px', 'padding-right': '0px', 'padding-bottom': '0px', 'padding-left': '0px',
        'margin-top': '0px', 'margin-right': '0px', 'margin-bottom': '0px', 'margin-left': '0px',
        'background-color': 'rgba(0, 0, 0, 0)', 'background-image': 'none', 'background-clip': 'border-box',
        'border-top-left-radius': '0px', 'border-top-right-radius': '0px', 'border-bottom-right-radius': '0px', 'border-bottom-left-radius': '0px',
        'box-shadow': 'none', opacity: '1', transform: 'none', 'mask-image': 'none', stroke: 'none', visibility: 'visible',
        'image-rendering': 'auto', 'pointer-events': 'auto', content: 'normal',
        'font-style': 'normal', 'letter-spacing': 'normal', 'font-variant-numeric': 'normal', 'text-transform': 'none',
        'text-decoration-line': 'none', 'text-align': 'start', 'white-space': 'normal', 'text-overflow': 'clip',
    };

    function convert(prop, value) {
        if (prop.endsWith('color') || prop === 'fill') {
            const color = parseColor(value);
            return color || value;
        }
        if (prop === 'box-shadow') return parseShadows(value);
        if (prop === 'font-weight' || prop === 'opacity' || prop === 'flex-grow' || prop === 'flex-shrink' || prop === 'z-index') {
            return Number.isNaN(Number(value)) ? value : Number(value);
        }
        if (prop.endsWith('-duration') || prop.endsWith('-delay')) return seconds(value);
        if (prop === 'transition-property' || prop === 'transition-timing-function' || prop === 'animation-name' || prop === 'animation-timing-function' ||
            prop === 'animation-iteration-count' || prop === 'animation-direction' || prop === 'animation-fill-mode' || prop === 'animation-play-state') {
            return splitList(value).map((part) => (part === 'infinite' || Number.isNaN(Number(part)) ? part : Number(part)));
        }
        return px(value);
    }

    function hasOwnText(element) {
        for (const node of element.childNodes) {
            if (node.nodeType === Node.TEXT_NODE && node.textContent.trim()) return true;
        }
        return element.matches('input, textarea');
    }

    function style(element, pseudo) {
        const computed = getComputedStyle(element, pseudo || null);
        const out = {};
        const isSvg = element instanceof SVGElement && !pseudo;
        const text = !pseudo && hasOwnText(element);
        const props = [...BOX_PROPS, ...LOOK_PROPS, ...(text ? TEXT_PROPS : []), ...MOTION_PROPS];
        for (const prop of props) {
            const value = computed.getPropertyValue(prop);
            if (value === '' || DEFAULTS[prop] === value) continue;
            if ((prop === 'top' || prop === 'right' || prop === 'bottom' || prop === 'left') && (computed.position === 'static' || value === 'auto')) continue;
            if (prop.startsWith('flex-direction') && !computed.display.includes('flex')) continue;
            if ((prop === 'width' || prop === 'height') && !pseudo) continue; // the box has them
            if (prop.startsWith('border-') && prop.endsWith('-width')) {
                const side = prop.split('-')[1];
                if (computed.getPropertyValue(`border-${side}-style`) === 'none') continue;
            }
            if (prop.startsWith('border-') && (prop.endsWith('-style') || prop.endsWith('-color')) && !prop.includes('radius')) {
                const side = prop.split('-')[1];
                if (computed.getPropertyValue(`border-${side}-style`) === 'none') continue;
            }
            if (prop.startsWith('outline-') && prop !== 'outline-style' && computed.outlineStyle === 'none') continue;
            if (prop === 'outline-style' && value === 'none') continue;
            if (prop === 'transform-origin' && computed.transform === 'none' && computed.animationName === 'none') continue;
            if ((prop === 'fill') && !isSvg) continue;
            if (prop === 'cursor' && value === 'auto') continue;
            if (prop === 'user-select' && value === 'auto') continue;
            if (prop.startsWith('transition-') && computed.transitionDuration.split(',').every((d) => parseFloat(d) === 0)) continue;
            if (prop.startsWith('animation-') && computed.animationName === 'none') continue;
            if (prop === 'content' && !pseudo) continue;
            if ((prop === 'min-width' || prop === 'min-height') && value === '0px') continue;
            if (prop === 'flex-direction' && value === 'row') continue;
            if (computed.position === 'relative' && ['top', 'right', 'bottom', 'left'].includes(prop) && value === '0px') continue;
            out[prop] = convert(prop, value);
        }
        // Inherited values equal to the parent's are left out, except the color and font of
        // elements that draw text or icons.
        const parentElement = pseudo ? element : element.parentElement;
        if (parentElement) {
            const parent = getComputedStyle(parentElement);
            for (const prop of ['cursor', 'user-select', 'visibility', 'pointer-events']) {
                if (prop in out && parent.getPropertyValue(prop) === computed.getPropertyValue(prop)) delete out[prop];
            }
        }
        return compact(out);
    }

    // Four sides as one value when equal, else [top, right, bottom, left]; gap likewise.
    function compact(out) {
        const groups = [
            ['padding', ['padding-top', 'padding-right', 'padding-bottom', 'padding-left']],
            ['margin', ['margin-top', 'margin-right', 'margin-bottom', 'margin-left']],
            ['border-width', ['border-top-width', 'border-right-width', 'border-bottom-width', 'border-left-width']],
            ['border-style', ['border-top-style', 'border-right-style', 'border-bottom-style', 'border-left-style']],
            ['border-color', ['border-top-color', 'border-right-color', 'border-bottom-color', 'border-left-color']],
            ['border-radius', ['border-top-left-radius', 'border-top-right-radius', 'border-bottom-right-radius', 'border-bottom-left-radius']],
            ['overflow', ['overflow-x', 'overflow-y']],
            ['gap', ['row-gap', 'column-gap']],
        ];
        const result = {};
        const grouped = new Set();
        for (const [name, props] of groups) {
            if (!props.some((prop) => prop in out)) continue;
            const zero = name === 'border-color' || name === 'border-style' ? null : name === 'overflow' ? 'visible' : name === 'gap' ? 'normal' : 0;
            const values = props.map((prop) => (prop in out ? out[prop] : zero));
            props.forEach((prop) => grouped.add(prop));
            result[name] = values.every((value) => JSON.stringify(value) === JSON.stringify(values[0])) ? values[0] : values;
        }
        const ordered = {};
        for (const [key, value] of Object.entries(out)) {
            if (grouped.has(key)) {
                const group = groups.find(([, props]) => props.includes(key))[0];
                if (!(group in ordered)) ordered[group] = result[group];
            } else {
                ordered[key] = value;
            }
        }
        return ordered;
    }

    function nameOf(element) {
        if (names.has(element)) return names.get(element);
        let name;
        if (element === document.documentElement) name = 'html';
        else if (element === document.body) name = 'body';
        else if (element.id) name = '#' + element.id;
        else {
            const parent = element.parentElement;
            const base = element.classList.length ? '.' + element.classList[0] : element.tagName.toLowerCase();
            const same = [...parent.children].filter((child) => (child.classList.length ? '.' + child.classList[0] : child.tagName.toLowerCase()) === base && !child.id);
            name = nameOf(parent) + '>' + base + (same.length > 1 ? `[${same.indexOf(element)}]` : '');
        }
        names.set(element, name);
        return name;
    }

    function box(rect) {
        return [round(rect.left), round(rect.top), round(rect.width), round(rect.height)];
    }

    // The visible part of a rect, inside every clipping ancestor and the viewport.
    function clipOf(element) {
        let clip = [0, 0, innerWidth, innerHeight];
        for (let node = element.parentElement; node; node = node.parentElement) {
            const computed = getComputedStyle(node);
            if (computed.overflowX === 'visible' && computed.overflowY === 'visible') continue;
            const rect = node.getBoundingClientRect();
            const inner = [rect.left + node.clientLeft, rect.top + node.clientTop, node.clientWidth, node.clientHeight];
            clip = intersect(clip, inner);
        }
        return clip;
    }

    function intersect(a, b) {
        const x = Math.max(a[0], b[0]);
        const y = Math.max(a[1], b[1]);
        const right = Math.min(a[0] + a[2], b[0] + b[2]);
        const bottom = Math.min(a[1] + a[3], b[1] + b[3]);
        return [x, y, Math.max(0, right - x), Math.max(0, bottom - y)];
    }

    const ATTRS = ['role', 'title', 'aria-label', 'aria-pressed', 'aria-expanded', 'aria-selected', 'aria-disabled', 'aria-valuenow', 'aria-valuemin', 'aria-valuemax', 'tabindex', 'placeholder', 'type', 'inert'];

    function skip(element) {
        return element.matches('script, style, link, meta, title, head, svg.icons, svg *');
    }

    window.__refName = nameOf;
    window.__refStyle = (element, pseudo) => style(element, pseudo);

    window.__refCollect = function () {
        const elements = [];
        const hidden = [];
        const text = [];
        const pseudo = [];
        const scroll = [];
        const walk = (element) => {
            if (skip(element)) return;
            const computed = getComputedStyle(element);
            const name = nameOf(element);
            if (computed.display === 'none') {
                hidden.push(name);
                return;
            }
            const rect = element.getBoundingClientRect();
            const item = { name, tag: element.tagName.toLowerCase(), box: box(rect) };
            if (element.className && typeof element.className === 'string') item.class = element.className;
            else if (element instanceof SVGElement && element.getAttribute('class')) item.class = element.getAttribute('class');
            const attrs = {};
            for (const attr of ATTRS) {
                if (element.hasAttribute(attr)) attrs[attr] = element.getAttribute(attr);
            }
            if (element.disabled) attrs.disabled = true;
            if (element.readOnly) attrs.readonly = true;
            if (element.type === 'checkbox') attrs.checked = element.checked;
            if (element.matches('input')) attrs.value = element.value;
            if (Object.keys(attrs).length) item.attrs = attrs;
            if (element.tagName.toLowerCase() === 'svg') {
                const use = element.querySelector('use');
                if (use) item.icon = use.getAttribute('href');
            }
            if (hasOwnText(element) && !element.matches('input')) item.text = element.textContent.replace(/\s+/g, ' ').trim();
            if (element.scrollWidth > element.clientWidth + 0.5 && computed.overflowX !== 'visible' && computed.textOverflow === 'ellipsis') item.truncated = true;
            if (element === document.activeElement && element !== document.body) item.focused = true;
            item.style = style(element);
            const clip = clipOf(element);
            const visible = intersect(item.box, clip);
            if (visible[2] < item.box[2] - 0.5 || visible[3] < item.box[3] - 0.5) item.visible = visible.map((v) => round(v));
            if ((computed.overflowY === 'auto' || computed.overflowY === 'scroll') && element.scrollHeight > element.clientHeight) {
                scroll.push({ name, scrollTop: element.scrollTop, scrollHeight: element.scrollHeight, clientHeight: element.clientHeight });
            }
            for (const which of ['::before', '::after']) {
                const pseudoStyle = getComputedStyle(element, which);
                if (pseudoStyle.content !== 'none' && pseudoStyle.content !== 'normal' && pseudoStyle.display !== 'none') pseudo.push(name + which);
            }
            elements.push(item);

            // Text runs: what the comparison tool masks and compares by box and color.
            for (const node of element.childNodes) {
                if (node.nodeType !== Node.TEXT_NODE || !node.textContent.trim()) continue;
                const range = document.createRange();
                range.selectNodeContents(node);
                const rects = [...range.getClientRects()].filter((r) => r.width > 0 && r.height > 0).map(box);
                if (!rects.length) continue;
                text.push({
                    element: name,
                    text: node.textContent.replace(/\s+/g, ' ').trim(),
                    rects,
                    clip: clip.map((v) => round(v)),
                    color: parseColor(computed.color),
                    font: { family: computed.fontFamily, size: parseFloat(computed.fontSize), weight: Number(computed.fontWeight), lineHeight: px(computed.lineHeight) },
                });
            }
            if (element.matches('input') && (element.value || element.placeholder)) {
                const inner = [rect.left + element.clientLeft, rect.top + element.clientTop, element.clientWidth, element.clientHeight].map((v) => round(v));
                text.push({
                    element: name,
                    text: element.value || element.placeholder,
                    placeholder: !element.value,
                    rects: [inner],
                    clip: inner,
                    color: parseColor(element.value ? computed.color : getComputedStyle(element, '::placeholder').color),
                    font: { family: computed.fontFamily, size: parseFloat(computed.fontSize), weight: Number(computed.fontWeight), lineHeight: px(computed.lineHeight) },
                    input: true,
                });
            }
            for (const child of element.children) walk(child);
        };
        walk(document.documentElement);

        const animations = document.getAnimations().map((animation) => {
            const effect = animation.effect;
            const timing = effect.getComputedTiming();
            const target = effect.target ? nameOf(effect.target) + (effect.pseudoElement || '') : null;
            return {
                target,
                type: animation.constructor.name,
                name: animation.animationName || animation.transitionProperty || animation.id || null,
                duration: round(timing.duration, 3),
                delay: round(timing.delay, 3),
                iterations: timing.iterations === Infinity ? 'infinite' : timing.iterations,
                direction: timing.direction,
                easing: timing.easing,
                fill: timing.fill,
                currentTime: animation.currentTime === null ? null : round(animation.currentTime, 3),
                playState: animation.playState,
                keyframes: effect.getKeyframes().map((frame) => {
                    const out = {};
                    for (const [key, value] of Object.entries(frame)) {
                        if (key === 'computedOffset') continue;
                        out[key] = value;
                    }
                    return out;
                }),
            };
        });

        const docScroll = document.scrollingElement;
        if (docScroll.scrollHeight > docScroll.clientHeight) {
            scroll.push({ name: 'document', scrollTop: docScroll.scrollTop, scrollHeight: docScroll.scrollHeight, clientHeight: docScroll.clientHeight });
        }
        for (const element of [document.body]) {
            if (getComputedStyle(element).overflowY === 'auto' && element.scrollHeight > element.clientHeight && !scroll.some((s) => s.name === 'body')) {
                scroll.push({ name: 'body', scrollTop: element.scrollTop, scrollHeight: element.scrollHeight, clientHeight: element.clientHeight });
            }
        }
        return { viewport: [innerWidth, innerHeight], devicePixelRatio, elements, hidden, text, pseudo, animations, scroll };
    };
})();
