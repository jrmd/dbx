import { Download } from "@/components/site/download";
import { Features } from "@/components/site/features";
import { Footer } from "@/components/site/footer";
import { Hero } from "@/components/site/hero";
import { Nav } from "@/components/site/nav";
import { Query } from "@/components/site/query";
import { Safety } from "@/components/site/safety";
import { Statement } from "@/components/site/statement";
import { Vault } from "@/components/site/vault";
import { getLatestRelease } from "@/lib/release";

export const revalidate = 3600;

export default async function Home() {
  const release = await getLatestRelease();

  return (
    <>
      <Nav />
      <main className="flex-1">
        <Hero release={release} />
        <Statement />
        <Features />
        <Query />
        <Safety />
        <Vault />
        <Download release={release} />
      </main>
      <Footer />
    </>
  );
}
