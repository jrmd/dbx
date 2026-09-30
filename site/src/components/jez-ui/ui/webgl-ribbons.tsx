"use client";
import * as React from "react";
import { useFrame, useThree } from "@react-three/fiber";
import * as THREE from "three";

// The surface and its normal are evaluated together on the GPU. Geometry stays
// immutable; there are no per-frame buffers, React updates, or texture requests.
const vertex = /* glsl */ `
uniform float time;
uniform float aspect;
uniform float strand;
uniform float orbit;
uniform vec2 pointer;
varying vec3 vNormal;
varying vec3 vPosition;
varying vec2 vUv;
vec3 surface(vec2 p) {
  float u = p.x;
  float phase = strand * .19;
  float wave = u * 6.28318 + time * .24 + phase;
  float width = (.16 + .22 * pow(sin(u * 3.14159), 2.)) * (1. + strand * .045);
  float twist = wave * .72 + strand * .32;
  vec3 center = vec3(
    (u - .5) * max(6., aspect * 5.4),
    sin(wave) * .86 + (strand - 3.) * .28,
    cos(wave + strand * .23) * .58
  );
  center.y += sin(u * 3.14159) * pointer.y * .4;
  center.z += sin(u * 3.14159) * pointer.x * .5;
  if (orbit > .5) {
    float angle = u * 6.28318;
    float radius = 1.1 + strand * .12;
    center = vec3(cos(angle) * radius, sin(angle) * radius * .65, sin(angle * 2. + time * .24 + phase) * .55);
    twist = angle * 2. + phase + time * .08;
  }
  return center + vec3(0., cos(twist), sin(twist)) * (p.y - .5) * width;
}
void main() {
  vUv = uv;
  vec3 p = surface(uv);
  vec3 along = surface(uv + vec2(.0005, 0.)) - p;
  vec3 across = surface(uv + vec2(0., .0005)) - p;
  vNormal = normalize(normalMatrix * normalize(cross(along, across)));
  vec4 view = modelViewMatrix * vec4(p, 1.);
  vPosition = view.xyz;
  gl_Position = projectionMatrix * view;
}`;
const fragment = /* glsl */ `
uniform vec3 tint;
uniform float strand;
varying vec3 vNormal;
varying vec3 vPosition;
varying vec2 vUv;
void main() {
  vec3 n = normalize(vNormal) * (gl_FrontFacing ? 1. : -1.);
  vec3 eye = normalize(-vPosition);
  vec3 light = normalize(vec3(-.3, .9, 1.2));
  float diffuse = max(dot(n, light), 0.);
  float rim = pow(1. - abs(dot(n, eye)), 3.);
  float specular = pow(max(dot(n, normalize(light + eye)), 0.), 42.);
  float fold = pow(max(dot(n, normalize(vec3(.4, -.8, .9))), 0.), 8.);
  vec3 base = mix(tint, vec3(.82, .88, .92), mod(strand, 3.) * .19);
  vec3 color = base * (.13 + diffuse * .66) + vec3(.93, .97, 1.) * specular * .8;
  color += base * fold * .25 + vec3(.7, .83, .94) * rim * .28;
  // Fine longitudinal highlights suggest a surface, without a texture asset.
  color *= .97 + .03 * sin(vUv.y * 210.);
  gl_FragColor = vec4(color, 1.);
  #include <tonemapping_fragment>
  #include <colorspace_fragment>
}`;

function Ribbon({
  index,
  color,
  speed,
  orbit,
}: {
  index: number;
  color: string;
  speed: number;
  orbit: boolean;
}) {
  const { size } = useThree();
  const material = React.useMemo(
    () =>
      new THREE.ShaderMaterial({
        vertexShader: vertex,
        fragmentShader: fragment,
        side: THREE.DoubleSide,
        uniforms: {
          time: { value: 0 },
          aspect: { value: 1 },
          strand: { value: index },
          orbit: { value: orbit ? 1 : 0 },
          pointer: { value: new THREE.Vector2() },
          tint: { value: new THREE.Color(color) },
        },
      }),
    [index, orbit, color],
  );
  React.useEffect(() => () => material.dispose(), [material]);
  material.uniforms.aspect.value = size.width / Math.max(size.height, 1);
  useFrame(({ pointer }, delta) => {
    material.uniforms.time.value += Math.min(delta, 0.05) * speed;
    material.uniforms.pointer.value.lerp(pointer, 1 - Math.exp(-delta * 3));
  });
  return (
    <mesh material={material} frustumCulled={false}>
      <planeGeometry args={[1, 1, 192, 6]} />
    </mesh>
  );
}

export function RibbonScene({
  color,
  speed,
  composition,
}: {
  color: string;
  speed: number;
  composition?: "fold" | "orbit";
}) {
  return (
    <group rotation={[0, 0, -0.16]}>
      {Array.from({ length: 7 }, (_, i) => (
        <Ribbon
          key={i}
          index={i}
          color={color}
          speed={speed}
          orbit={composition === "orbit"}
        />
      ))}
    </group>
  );
}

export function RibbonPoster({ color }: { color: string }) {
  const id = React.useId().replaceAll(":", "");
  return (
    <svg
      viewBox="0 0 1200 480"
      preserveAspectRatio="xMidYMid slice"
      className="h-full w-full"
      aria-hidden="true"
    >
      <defs>
        <linearGradient id={id} x1="0" y1="0" x2="1" y2="1">
          <stop stopColor={color} stopOpacity=".3" />
          <stop offset=".45" stopColor={color} />
          <stop offset=".7" stopColor="#e1edf4" />
          <stop offset="1" stopColor={color} stopOpacity=".4" />
        </linearGradient>
      </defs>
      {Array.from({ length: 7 }, (_, i) => (
        <path
          key={i}
          d={`M -100 ${180 + i * 25} C 170 ${-120 + i * 12}, 310 ${570 - i * 14}, 610 ${290 + i * 14} S 960 ${70 + i * 12}, 1320 ${240 + i * 22}`}
          fill="none"
          stroke={`url(#${id})`}
          strokeWidth={12 + i * 2}
        />
      ))}
    </svg>
  );
}
