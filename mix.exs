defmodule ExSrpPhat.MixProject do
  use Mix.Project

  @version "0.1.0"
  @source_url "https://github.com/cortfritz/ex_srp_phat"

  def project do
    [
      app: :ex_srp_phat,
      version: @version,
      elixir: "~> 1.15",
      start_permanent: Mix.env() == :prod,
      deps: deps(),
      description: description(),
      package: package(),
      docs: docs(),
      source_url: @source_url
    ]
  end

  def application do
    [
      extra_applications: [:logger]
    ]
  end

  defp deps do
    [
      {:rustler, "~> 0.38", runtime: false, optional: true},
      {:rustler_precompiled, "~> 0.9"},
      {:ex_doc, "~> 0.34", only: :dev, runtime: false}
    ]
  end

  defp description do
    "Elixir NIF bindings for a Rust GCC-PHAT → SRP-PHAT acoustic source " <>
      "localizer over a known WGS-84 ECEF microphone-array geometry."
  end

  defp package do
    [
      name: "ex_srp_phat",
      files: [
        "lib",
        "native/srp_phat/src",
        "native/srp_phat/.cargo",
        "native/srp_phat/Cargo.toml",
        "native/srp_phat/Cargo.lock",
        "checksum-*.exs",
        "mix.exs",
        "README.md",
        "CHANGELOG.md",
        "LICENSE*"
      ],
      maintainers: ["Cort Fritz"],
      licenses: ["MIT"],
      links: %{
        "GitHub" => @source_url,
        "Changelog" => "#{@source_url}/blob/main/CHANGELOG.md"
      }
    ]
  end

  defp docs do
    [
      main: "ExSrpPhat",
      source_ref: "v#{@version}",
      extras: ["README.md", "CHANGELOG.md"]
    ]
  end
end
