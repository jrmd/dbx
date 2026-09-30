"use client";
import * as React from "react";
import { useFrame, useThree } from "@react-three/fiber";
import * as THREE from "three";
import { RibbonScene } from "./webgl-ribbons";
import {
  screenVertex,
  orbFragment,
  liquidFragment,
  distortionFragment,
  terrainFragment,
  terrainVertex,
  terrainReliefFragment,
  particleVertex,
  particleFragment,
} from "./webgl-shaders";
import { AtmosphereScene } from "./webgl-atmospheres";
export type SceneKind =
  | "silk"
  | "eclipse"
  | "tunnel"
  | "constellation"
  | "particles"
  | "ribbons"
  | "liquid"
  | "orb"
  | "terrain-relief"
  | "terrain"
  | "distortion";
export type SceneProps = {
  kind: SceneKind;
  color: string;
  speed: number;
  text?: string;
  imageSrc?: string;
  composition?: "fold" | "orbit";
};
function Field({ color, speed }: { color: string; speed: number }) {
  const material = React.useMemo(
    () =>
      new THREE.ShaderMaterial({
        vertexShader: particleVertex,
        fragmentShader: particleFragment,
        uniforms: {
          time: { value: 0 },
          pointer: { value: new THREE.Vector2() },
          tint: { value: new THREE.Color(color) },
        },
        transparent: true,
        depthWrite: false,
        blending: THREE.AdditiveBlending,
      }),
    [color],
  );
  const geometry = React.useMemo(() => {
    const n = 18000,
      p = new Float32Array(n * 3),
      s = new Float32Array(n);
    for (let i = 0; i < n; i++) {
      const a = i * 2.39996323;
      const r = 0.7 + Math.pow((i + 0.5) / n, 0.7) * 1.6;
      const warp = Math.sin(a * 3) * 0.18;
      const seed = (Math.sin(i * 127.1) * 43758.5453) % 1;
      const scatter = Math.abs(seed);
      p[i * 3] = Math.cos(a) * r;
      p[i * 3 + 1] = Math.sin(a) * r * 0.6 + Math.sin(r * 3) * 0.12;
      p[i * 3 + 2] =
        Math.sin(a * 2 + r * 3) * 0.42 + warp + (scatter - 0.5) * 0.12;
      s[i] = scatter;
    }
    const g = new THREE.BufferGeometry();
    g.setAttribute("position", new THREE.BufferAttribute(p, 3));
    g.setAttribute("seed", new THREE.BufferAttribute(s, 1));
    return g;
  }, []);
  React.useEffect(
    () => () => {
      geometry.dispose();
      material.dispose();
    },
    [geometry, material],
  );
  useFrame(({ pointer }, d) => {
    material.uniforms.time.value += Math.min(d, 0.05) * speed;
    material.uniforms.pointer.value.lerp(pointer, 0.04);
  });
  return (
    <points
      geometry={geometry}
      material={material}
      rotation={[0.18, 0, -0.25]}
    />
  );
}
function Surface({ kind, color, speed, imageSrc, text = "FORM" }: SceneProps) {
  const { size, invalidate } = useThree();
  const ref = React.useRef<THREE.Mesh>(null);
  const material = React.useMemo(
    () =>
      new THREE.ShaderMaterial({
        vertexShader: kind === "terrain-relief" ? terrainVertex : screenVertex,
        fragmentShader:
          kind === "orb"
            ? orbFragment
            : kind === "liquid"
              ? liquidFragment
              : kind === "terrain-relief"
                ? terrainReliefFragment
                : kind === "terrain"
                  ? terrainFragment
                  : distortionFragment,
        uniforms: {
          time: { value: 0 },
          aspect: { value: 1 },
          pointer: { value: new THREE.Vector2() },
          tint: { value: new THREE.Color(color) },
          picture: { value: null },
          imageAspect: { value: 1.5 },
        },
        side: THREE.DoubleSide,
      }),
    [kind, color],
  );
  React.useEffect(() => () => material.dispose(), [material]);
  React.useEffect(() => {
    material.uniforms.aspect.value = size.width / Math.max(1, size.height);
    invalidate();
  }, [material, size, invalidate]);
  React.useEffect(() => {
    if (kind !== "distortion") return;
    let cancelled = false;
    const canvas = document.createElement("canvas");
    canvas.width = 900;
    canvas.height = 600;
    const ctx = canvas.getContext("2d")!;
    ctx.fillStyle = "#dadfcf";
    ctx.fillRect(0, 0, 900, 600);
    ctx.fillStyle = "#28352d";
    ctx.font = "bold 190px sans-serif";
    const width = ctx.measureText(text).width;
    ctx.font = `bold ${Math.min(190, (190 * 680) / Math.max(width, 1))}px sans-serif`;
    ctx.fillText(text, 105, 350);
    ctx.fillStyle = "#c36943";
    ctx.fillRect(105, 405, 680, 14);
    let texture: THREE.Texture = new THREE.CanvasTexture(canvas);
    material.uniforms.picture.value = texture;
    invalidate();
    if (imageSrc)
      new THREE.TextureLoader().load(
        imageSrc,
        (t) => {
          if (cancelled) {
            t.dispose();
            return;
          }
          texture.dispose();
          texture = t;
          material.uniforms.picture.value = t;
          material.uniforms.imageAspect.value = t.image.width / t.image.height;
          invalidate();
        },
        undefined,
        () => {
          invalidate();
        },
      );
    return () => {
      cancelled = true;
      texture.dispose();
    };
  }, [kind, imageSrc, text, material, invalidate]);
  useFrame(({ pointer }, d) => {
    material.uniforms.time.value += Math.min(d, 0.05) * speed;
    material.uniforms.pointer.value.lerp(pointer, 0.06);
  });
  return (
    <mesh
      ref={ref}
      material={material}
      rotation={kind === "terrain-relief" ? [-0.75, 0, 0] : [0, 0, 0]}
    >
      {kind === "terrain-relief" ? (
        <planeGeometry
          args={[
            Math.max(14, (size.width / Math.max(size.height, 1)) * 7),
            10,
            280,
            180,
          ]}
        />
      ) : (
        <planeGeometry args={[2, 2]} />
      )}
    </mesh>
  );
}
export function WebGLScene(props: SceneProps) {
  if (
    props.kind === "silk" ||
    props.kind === "eclipse" ||
    props.kind === "tunnel" ||
    props.kind === "constellation"
  )
    return <AtmosphereScene {...props} kind={props.kind} />;
  if (props.kind === "ribbons") return <RibbonScene {...props} />;
  if (props.kind === "particles") return <Field {...props} />;
  return <Surface {...props} />;
}
